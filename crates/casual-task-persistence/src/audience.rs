//! Who could be notified about an event, and who may see it at all
//! (`docs/29` §Delivery).
//!
//! # The failure this module prevents
//!
//! Leaking a task into somebody's inbox. `docs/29`: "Recipient computation is
//! **permission-checked**. A user is never notified about a task they cannot
//! see — including via a mention. Mentioning someone in a private project does
//! not silently leak the task title into their inbox."
//!
//! That is why [`visible_to`] exists as a separate, mandatory step rather than
//! as a filter the fan-out could forget: a mention is a user id supplied by the
//! *client*, and without this check anyone could put any user id in a comment
//! on a private task and mail them its title.
//!
//! # Why the candidates are queried and the decision is not made here
//!
//! This module answers "who is connected to this task, and how". It does not
//! decide who is notified — self-suppression and rank resolution are
//! `casual-task-notification`'s, and they are pure so they can be tested
//! without a database. Splitting it this way is also what keeps the reason set
//! changeable without touching SQL.

use uuid::Uuid;

use crate::scoped::Scoped;

/// A person connected to a task, and the connection.
///
/// The `reason` is the stored spelling from
/// `casual_task_notification::Reason::as_str`; this crate may not depend on
/// that one (`docs/19` puts the domain above persistence), so the two meet as
/// text and the fan-out parses it back.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CandidateRow {
    pub user_id: Uuid,
    pub reason: &'static str,
}

/// Everyone assigned to a task. `docs/29`: reason `ASSIGNED`, rank 2.
///
/// # Errors
///
/// Any database error.
pub async fn assignees(scoped: &mut Scoped<'_>, task_id: Uuid) -> Result<Vec<Uuid>, sqlx::Error> {
    sqlx::query_scalar("SELECT user_id FROM task_assignee WHERE task_id = $1")
        .bind(task_id)
        .fetch_all(scoped.conn())
        .await
}

/// Who filed the task. `docs/29`: reason `REPORTED`, rank 3.
///
/// # Errors
///
/// Any database error.
pub async fn reporter(scoped: &mut Scoped<'_>, task_id: Uuid) -> Result<Option<Uuid>, sqlx::Error> {
    sqlx::query_scalar("SELECT reporter_id FROM task WHERE id = $1 AND deleted_at IS NULL")
        .bind(task_id)
        .fetch_optional(scoped.conn())
        .await
}

/// Everyone who has commented on the task. `docs/29`: reason `PARTICIPATED`,
/// rank 5.
///
/// Bounded: a task with two thousand comments must not turn one event into two
/// thousand candidate rows and a fan-out that runs for a minute. The bound is
/// the most recent commenters, because participation decays — someone who
/// commented once a year ago is not who `docs/29` means by "you commented on
/// it before".
///
/// # Errors
///
/// Any database error.
pub async fn participants(
    scoped: &mut Scoped<'_>,
    task_id: Uuid,
    limit: i64,
) -> Result<Vec<Uuid>, sqlx::Error> {
    sqlx::query_scalar(
        "SELECT author_id
           FROM comment
          WHERE task_id = $1 AND deleted_at IS NULL
          GROUP BY author_id
          ORDER BY max(created_at) DESC
          LIMIT $2",
    )
    .bind(task_id)
    .bind(limit)
    .fetch_all(scoped.conn())
    .await
}

/// The users a comment mentions, and the task it is on.
///
/// `migrations/0006` stores `mentions uuid[]` "resolved at write time", so the
/// fan-out reads ids rather than parsing prose — but they are ids a *client*
/// supplied, which is exactly why [`visible_to`] is not optional.
///
/// # Errors
///
/// Any database error.
pub async fn comment_mentions(
    scoped: &mut Scoped<'_>,
    comment_id: Uuid,
) -> Result<Option<(Uuid, Vec<Uuid>)>, sqlx::Error> {
    sqlx::query_as(
        "SELECT task_id, mentions
           FROM comment
          WHERE id = $1 AND deleted_at IS NULL",
    )
    .bind(comment_id)
    .fetch_optional(scoped.conn())
    .await
}

/// Narrow `candidates` to those who may actually see `task_id`.
///
/// `docs/04` §Visibility vs permission, applied per candidate: a workspace-wide
/// project, a team project whose team they are in, an explicit project
/// membership, or a project-scoped grant. Plus workspace membership, without
/// which a removed colleague would keep receiving mail about work they can no
/// longer open.
///
/// Returns the subset, in no particular order. An empty result is normal and
/// means nobody may be told.
///
/// # Errors
///
/// Any database error.
pub async fn visible_to(
    scoped: &mut Scoped<'_>,
    task_id: Uuid,
    candidates: &[Uuid],
) -> Result<Vec<Uuid>, sqlx::Error> {
    if candidates.is_empty() {
        return Ok(Vec::new());
    }
    let workspace = scoped.workspace_id().as_uuid();
    sqlx::query_scalar(
        "SELECT c.id
           FROM unnest($2::uuid[]) AS c(id)
           JOIN task t ON t.id = $1
           JOIN project p ON p.id = t.project_id
          WHERE t.workspace_id = $3
            AND t.deleted_at IS NULL
            AND p.deleted_at IS NULL
            -- Membership of the workspace is the floor. Everything below is a
            -- route INTO a project; none of them is a route into the tenant.
            AND EXISTS (SELECT 1 FROM workspace_membership wm
                         WHERE wm.workspace_id = $3 AND wm.user_id = c.id)
            AND (   p.visibility = 'WORKSPACE'
                 OR (p.visibility = 'TEAM'
                     AND EXISTS (SELECT 1 FROM team_membership tm
                                  WHERE tm.team_id = p.team_id AND tm.user_id = c.id))
                 OR EXISTS (SELECT 1 FROM project_membership pm
                             WHERE pm.project_id = p.id AND pm.user_id = c.id)
                 OR EXISTS (SELECT 1 FROM role_assignment ra
                             WHERE ra.workspace_id = $3
                               AND ra.principal_type = 'USER'::principal_type
                               AND ra.principal_id = c.id
                               AND ra.scope_type = 'PROJECT'::scope_type
                               AND ra.scope_id = p.id))",
    )
    .bind(task_id)
    .bind(candidates)
    .bind(workspace)
    .fetch_all(scoped.conn())
    .await
}

/// What an email needs about the task, and about who caused the event.
#[derive(Debug, Clone)]
pub struct Dispatchable {
    /// `WR-125`.
    pub key: String,
    pub title: String,
    pub project_id: Uuid,
}

/// The task's human key and title, for the email subject.
///
/// Read once per event rather than once per recipient: `docs/04`'s list rule
/// applied to fan-out, and the difference between one query and one per person
/// on a task with forty watchers.
///
/// # Errors
///
/// Any database error.
pub async fn dispatchable(
    scoped: &mut Scoped<'_>,
    task_id: Uuid,
) -> Result<Option<Dispatchable>, sqlx::Error> {
    let row: Option<(String, i64, String, Uuid)> = sqlx::query_as(
        "SELECT p.key, t.number, t.title, t.project_id
           FROM task t
           JOIN project p ON p.id = t.project_id
          WHERE t.id = $1 AND t.deleted_at IS NULL",
    )
    .bind(task_id)
    .fetch_optional(scoped.conn())
    .await?;
    Ok(row.map(|(key, number, title, project_id)| Dispatchable {
        key: format!("{key}-{number}"),
        title,
        project_id,
    }))
}

/// The addresses and display names for a set of recipients.
///
/// Returns `(user_id, display_name, email)`. An anonymized account has a NULL
/// email (ADR-026) and is skipped by the caller rather than filtered here, so
/// the in-app row is still written for them — `docs/29` makes in-app the system
/// of record, and having no address is not a reason to lose the record.
///
/// # Errors
///
/// Any database error.
pub async fn addresses(
    scoped: &mut Scoped<'_>,
    users: &[Uuid],
) -> Result<Vec<(Uuid, String, Option<String>)>, sqlx::Error> {
    if users.is_empty() {
        return Ok(Vec::new());
    }
    sqlx::query_as(
        "SELECT id, display_name, email::text
           FROM user_account
          WHERE id = ANY($1) AND is_tombstone = false",
    )
    .bind(users)
    .fetch_all(scoped.conn())
    .await
}
