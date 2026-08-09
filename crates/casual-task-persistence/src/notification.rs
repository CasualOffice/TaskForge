//! The notification inbox (`docs/29` §The inbox).
//!
//! # The failure this module prevents
//!
//! A notification that exists only in an email. `docs/29` §Channels: "In-app is
//! the source of truth: every notification lands there regardless of other
//! channel settings, so nothing is ever *only* in an email someone deleted."
//!
//! [`record`] is therefore the first thing the fan-out does and the only one
//! that is allowed to fail the delivery. The email is sent afterwards, by the
//! worker, and its failure is logged rather than propagated — a relay that is
//! down must not cost the recipient the record.
//!
//! # Why coalescing lives in the write and not in the read
//!
//! `docs/29` rule 2: "Changes to the same task within 5 minutes collapse into
//! one notification ('Sarah made 4 changes'). Someone editing a task for two
//! minutes should not generate eight emails." Collapsing on read would still
//! have sent the eight emails. So the collapse happens here, at the moment the
//! second change arrives, and the caller is told which happened — because an
//! email is sent for an insert and not for a merge.

use time::OffsetDateTime;
use uuid::Uuid;

use crate::scoped::Scoped;

/// `docs/29` rule 2. Changes to one aggregate inside this window collapse.
pub const COALESCE_WINDOW: time::Duration = time::Duration::minutes(5);

/// A notification as stored.
#[derive(Debug, Clone)]
pub struct NotificationRow {
    pub id: Uuid,
    pub user_id: Uuid,
    pub event_type: String,
    pub reason: String,
    pub aggregate_id: Option<Uuid>,
    pub payload: serde_json::Value,
    pub created_at: OffsetDateTime,
    pub read_at: Option<OffsetDateTime>,
}

/// What [`record`] did, so the caller knows whether to send an email.
///
/// The distinction is the whole point of coalescing: a merge must not produce a
/// second email, and returning `()` here would leave the worker no way to tell.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Recorded {
    /// A new row. The recipient has not been told about this aggregate
    /// recently, so a channel delivery is due.
    Inserted(Uuid),
    /// An existing unread row absorbed this change (`docs/29` rule 2). No
    /// second email.
    Coalesced(Uuid),
}

impl Recorded {
    #[must_use]
    pub const fn id(&self) -> Uuid {
        match self {
            Self::Inserted(id) | Self::Coalesced(id) => *id,
        }
    }

    /// Whether a channel delivery is due for this.
    #[must_use]
    pub const fn is_new(&self) -> bool {
        matches!(self, Self::Inserted(_))
    }
}

/// What the fan-out has decided to tell one person.
#[derive(Debug, Clone)]
pub struct NewNotification<'a> {
    pub user_id: Uuid,
    pub event_type: &'a str,
    /// The stored spelling from `casual_task_notification::Reason::as_str`.
    pub reason: &'a str,
    pub aggregate_id: Uuid,
    pub payload: serde_json::Value,
}

/// Write the in-app record, collapsing into a recent unread one if there is one.
///
/// The reason on a coalesced row is raised, never lowered: being mentioned on a
/// task you were already notified about as its reporter must relabel the
/// notification `MENTIONED`, because that is the reason you would act on.
/// Lowering it would bury a direct address underneath a change you had already
/// decided to ignore.
///
/// `rank_of` is passed in rather than imported: this crate sits below
/// `casual-task-notification` in the dependency DAG `docs/19` fixes, so the
/// ranking arrives as a function and the reason as text.
///
/// # Errors
///
/// Any database error.
pub async fn record(
    scoped: &mut Scoped<'_>,
    new: &NewNotification<'_>,
    rank_of: impl Fn(&str) -> u8,
) -> Result<Recorded, sqlx::Error> {
    let workspace = scoped.workspace_id().as_uuid();

    // Served by notification_coalesce_ix (migration 0024). Without the index
    // this is a scan of the recipient's whole unread set, on every event in the
    // system.
    let existing: Option<(Uuid, String)> = sqlx::query_as(
        "SELECT id, reason
           FROM notification
          WHERE workspace_id = $1
            AND user_id = $2
            AND aggregate_id = $3
            AND read_at IS NULL
            AND created_at > now() - $4::interval
          ORDER BY created_at DESC
          LIMIT 1",
    )
    .bind(workspace)
    .bind(new.user_id)
    .bind(new.aggregate_id)
    .bind(pg_interval(COALESCE_WINDOW))
    .fetch_optional(scoped.conn())
    .await?;

    if let Some((id, current_reason)) = existing {
        // A lower rank number is a higher reason (`docs/29`).
        let reason = if rank_of(new.reason) < rank_of(&current_reason) {
            new.reason
        } else {
            current_reason.as_str()
        };
        sqlx::query(
            "UPDATE notification
                SET reason = $2,
                    event_type = $3,
                    payload = $4,
                    created_at = now()
              WHERE id = $1",
        )
        .bind(id)
        .bind(reason)
        .bind(new.event_type)
        .bind(&new.payload)
        .execute(scoped.conn())
        .await?;
        return Ok(Recorded::Coalesced(id));
    }

    let id = Uuid::now_v7();
    sqlx::query(
        "INSERT INTO notification
             (id, workspace_id, user_id, event_type, reason, aggregate_id, payload)
         VALUES ($1,$2,$3,$4,$5,$6,$7)",
    )
    .bind(id)
    .bind(workspace)
    .bind(new.user_id)
    .bind(new.event_type)
    .bind(new.reason)
    .bind(new.aggregate_id)
    .bind(&new.payload)
    .execute(scoped.conn())
    .await?;
    Ok(Recorded::Inserted(id))
}

/// The tuple a notification row decodes into before it becomes a
/// [`NotificationRow`]. Named because eight columns inline in a signature is
/// unreadable, and clippy is right about that.
type InboxTuple = (
    Uuid,
    Uuid,
    String,
    String,
    Option<Uuid>,
    serde_json::Value,
    OffsetDateTime,
    Option<OffsetDateTime>,
);

/// The cursor position for the inbox: `(is_unread, created_at, id)`.
///
/// The unread flag is part of the key because it is the leading sort column.
/// A cursor that carried only the timestamp would resume in the wrong section
/// as soon as the page crossed from unread into read.
pub type InboxCursor = (bool, OffsetDateTime, Uuid);

/// One page of a person's inbox, unread first, newest first within each part.
///
/// `limit` is the page size; **one more row than that is fetched**, so "is
/// there a next page" costs no second query (`docs/05` §Pagination).
///
/// # Errors
///
/// Any database error.
pub async fn inbox(
    scoped: &mut Scoped<'_>,
    user_id: Uuid,
    after: Option<InboxCursor>,
    limit: u32,
) -> Result<Vec<NotificationRow>, sqlx::Error> {
    let workspace = scoped.workspace_id().as_uuid();
    // The row-value comparison is a single `<` across all three keys because
    // all three are DESC. `docs/26`: PostgreSQL drives a composite index from
    // the row-value form and often cannot from the expanded one — and here the
    // composite is the expression index notification_inbox_ix (migration 0024).
    let rows: Vec<InboxTuple> = sqlx::query_as(
        "SELECT id, user_id, event_type, reason, aggregate_id, payload,
                created_at, read_at
           FROM notification
          WHERE workspace_id = $1
            AND user_id = $2
            AND ($3::boolean IS NULL
                 OR ((read_at IS NULL), created_at, id)
                     < ($3::boolean, $4::timestamptz, $5::uuid))
          ORDER BY (read_at IS NULL) DESC, created_at DESC, id DESC
          LIMIT $6",
    )
    .bind(workspace)
    .bind(user_id)
    .bind(after.map(|c| c.0))
    .bind(after.map(|c| c.1))
    .bind(after.map(|c| c.2))
    .bind(i64::from(limit).saturating_add(1))
    .fetch_all(scoped.conn())
    .await?;

    Ok(rows
        .into_iter()
        .map(
            |(id, user_id, event_type, reason, aggregate_id, payload, created_at, read_at)| {
                NotificationRow {
                    id,
                    user_id,
                    event_type,
                    reason,
                    aggregate_id,
                    payload,
                    created_at,
                    read_at,
                }
            },
        )
        .collect())
}

/// The unread badge. Served as an index-only count by `notification_unread_ix`,
/// not a scan (`docs/29` §The inbox).
///
/// # Errors
///
/// Any database error.
pub async fn unread_count(scoped: &mut Scoped<'_>, user_id: Uuid) -> Result<i64, sqlx::Error> {
    let workspace = scoped.workspace_id().as_uuid();
    sqlx::query_scalar(
        "SELECT count(*) FROM notification
          WHERE workspace_id = $1 AND user_id = $2 AND read_at IS NULL",
    )
    .bind(workspace)
    .bind(user_id)
    .fetch_one(scoped.conn())
    .await
}

/// Mark specific notifications read. Returns how many changed.
///
/// Scoped to `user_id` in the statement rather than checked first: read state is
/// per person, and a caller that passed somebody else's notification id must
/// affect nothing rather than be told it exists.
///
/// # Errors
///
/// Any database error.
pub async fn mark_read(
    scoped: &mut Scoped<'_>,
    user_id: Uuid,
    ids: &[Uuid],
) -> Result<u64, sqlx::Error> {
    let workspace = scoped.workspace_id().as_uuid();
    Ok(sqlx::query(
        "UPDATE notification SET read_at = now()
          WHERE workspace_id = $1 AND user_id = $2 AND id = ANY($3) AND read_at IS NULL",
    )
    .bind(workspace)
    .bind(user_id)
    .bind(ids)
    .execute(scoped.conn())
    .await?
    .rows_affected())
}

/// Mark everything read. Returns how many changed.
///
/// # Errors
///
/// Any database error.
pub async fn mark_all_read(scoped: &mut Scoped<'_>, user_id: Uuid) -> Result<u64, sqlx::Error> {
    let workspace = scoped.workspace_id().as_uuid();
    Ok(sqlx::query(
        "UPDATE notification SET read_at = now()
          WHERE workspace_id = $1 AND user_id = $2 AND read_at IS NULL",
    )
    .bind(workspace)
    .bind(user_id)
    .execute(scoped.conn())
    .await?
    .rows_affected())
}

/// A `time::Duration` as a PostgreSQL interval literal.
fn pg_interval(duration: time::Duration) -> String {
    format!("{} seconds", duration.whole_seconds())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_coalescing_window_is_the_one_docs_29_fixes() {
        // "Changes to the same task within 5 minutes collapse into one."
        assert_eq!(COALESCE_WINDOW, time::Duration::minutes(5));
        assert_eq!(pg_interval(COALESCE_WINDOW), "300 seconds");
    }

    #[test]
    fn an_insert_is_new_and_a_merge_is_not() {
        // The distinction the worker branches on. If `Coalesced` ever reported
        // itself as new, ten edits in two minutes would send ten emails — which
        // is the exact failure rule 2 exists to stop.
        let id = Uuid::now_v7();
        assert!(Recorded::Inserted(id).is_new());
        assert!(!Recorded::Coalesced(id).is_new());
        assert_eq!(Recorded::Coalesced(id).id(), id);
        assert_eq!(Recorded::Inserted(id).id(), id);
    }
}
