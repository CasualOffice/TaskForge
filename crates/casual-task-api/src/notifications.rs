//! `/api/v1/notifications` — the inbox (`docs/29` §The inbox).
//!
//! # The failure this module prevents
//!
//! Reading, or clearing, somebody else's inbox. Read state is per person
//! (`docs/29`: "Read state is per user and syncs across devices"), and a
//! notification is the one tenant row whose owner is not implied by the
//! workspace — every member of a workspace can see the same task, and exactly
//! one of them may see a given notification about it.
//!
//! So `user_id` is never taken from the request. It comes from
//! [`WorkspaceMember`], and it is a `WHERE` clause in every statement rather
//! than a check before one: a caller passing a stranger's notification id
//! affects zero rows and is told zero rows changed, which is the same answer
//! they get for an id that never existed.
//!
//! # No permission check beyond membership, and why that is right
//!
//! `docs/04` gives visibility an implicit read grant; a notification is not a
//! project resource, it is a record addressed to one person. Nothing in the
//! closed permission registry governs "may I read my own inbox", and inventing
//! one would be settling a design question in a handler.

use std::collections::HashMap;

use axum::extract::{Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use casual_task_model::Cursor;
use casual_task_persistence::notification::{self, InboxCursor, NotificationRow};
use serde::{Deserialize, Serialize};
use time::OffsetDateTime;
use time::format_description::well_known::Rfc3339;
use uuid::Uuid;

use crate::context::Context;
use crate::error::{ApiError, codes};
use crate::middleware::WorkspaceMember;
use crate::server::{AppState, RequestId};
use crate::unit;
use crate::wire::{self, Body, Page, Paged};

/// A notification, as a client sees it.
#[derive(Debug, Serialize)]
pub struct NotificationView {
    pub id: Uuid,
    pub event_type: String,
    /// The single highest-ranked reason (`docs/29`).
    pub reason: String,
    /// The task this is about.
    pub aggregate_id: Option<Uuid>,
    pub payload: serde_json::Value,
    pub created_at: String,
    /// `null` while unread. The badge counts these.
    pub read_at: Option<String>,
}

impl From<&NotificationRow> for NotificationView {
    fn from(row: &NotificationRow) -> Self {
        Self {
            id: row.id,
            event_type: row.event_type.clone(),
            reason: row.reason.clone(),
            aggregate_id: row.aggregate_id,
            payload: row.payload.clone(),
            created_at: wire::timestamp(row.created_at),
            read_at: row.read_at.map(wire::timestamp),
        }
    }
}

/// The inbox page, with the badge alongside it.
///
/// `unread_count` rides on the list response because every client that renders
/// an inbox renders the badge beside it, and a second round trip for a number
/// the same transaction already has is a round trip nobody needs.
#[derive(Debug, Serialize)]
pub struct Inbox {
    #[serde(flatten)]
    pub page: Paged<NotificationView>,
    pub unread_count: i64,
}

/// `POST /api/v1/notifications/read`.
///
/// Exactly one of the two fields, enforced below. A body carrying both is a
/// client that has not decided what it wants, and answering it by picking one
/// silently is how "mark all read" becomes a support ticket.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MarkRead {
    #[serde(default)]
    pub ids: Option<Vec<Uuid>>,
    #[serde(default)]
    pub all: Option<bool>,
}

/// How many ids one request may mark. `docs/21` bounds every input.
const MAX_IDS: usize = 200;

#[derive(Debug, Serialize)]
pub struct MarkReadResult {
    /// How many were unread and are now read. A second call marks zero, which
    /// is the correct answer rather than an error.
    pub marked: u64,
    pub unread_count: i64,
}

/// `GET /api/v1/notifications` — unread first, newest first, cursor-paged.
///
/// # Errors
///
/// `400` for an unknown query parameter, a bad cursor, or an over-limit page.
pub async fn list(
    State(state): State<AppState>,
    member: WorkspaceMember,
    headers: HeaderMap,
    Query(params): Query<HashMap<String, String>>,
) -> Result<Response, ApiError> {
    let request_id = RequestId::of_parts(&headers);
    unit::reject_unknown(&params, &["limit", "cursor"], &request_id)?;
    let limit = page_size(&params, &request_id)?;
    let after = inbox_cursor(&params, &request_id)?;

    let mut tx = unit::begin(&state, &request_id).await?;
    let mut scoped = unit::scope(&mut tx, &member, &request_id).await?;
    let ctx = Context::load(&mut scoped, &member, &headers, &request_id).await?;

    let mut rows = notification::inbox(&mut scoped, ctx.actor.as_uuid(), after, limit)
        .await
        .map_err(|error| {
            tracing::error!(%error, "reading the inbox failed");
            ApiError::internal(&request_id)
        })?;
    let unread_count = notification::unread_count(&mut scoped, ctx.actor.as_uuid())
        .await
        .map_err(|error| {
            tracing::error!(%error, "counting unread notifications failed");
            ApiError::internal(&request_id)
        })?;
    unit::commit(tx, &request_id).await?;

    // One more row than asked for was fetched; its existence is the answer to
    // "is there a next page", and it is not part of this one.
    let has_more = rows.len() > limit as usize;
    rows.truncate(limit as usize);
    let next_cursor = has_more.then(|| rows.last()).flatten().map(encode_cursor);

    Ok(axum::Json(Inbox {
        page: Paged {
            data: rows.iter().map(NotificationView::from).collect(),
            page: Page {
                next_cursor,
                has_more,
            },
        },
        unread_count,
    })
    .into_response())
}

/// `POST /api/v1/notifications/read`.
///
/// # Errors
///
/// `400` when neither or both of `ids` and `all` are given, or too many ids.
pub async fn mark_read(
    State(state): State<AppState>,
    member: WorkspaceMember,
    headers: HeaderMap,
    Body(body): Body<MarkRead>,
) -> Result<Response, ApiError> {
    let request_id = RequestId::of_parts(&headers);
    let target = Target::of(&body, &request_id)?;

    let mut tx = unit::begin(&state, &request_id).await?;
    let mut scoped = unit::scope(&mut tx, &member, &request_id).await?;
    let ctx = Context::load(&mut scoped, &member, &headers, &request_id).await?;
    let actor = ctx.actor.as_uuid();

    // `actor`, never a user id from the body. Read state is per person, and the
    // only person a request may mark read for is the one who made it.
    let marked = match target {
        Target::All => notification::mark_all_read(&mut scoped, actor).await,
        Target::Ids(ids) => notification::mark_read(&mut scoped, actor, &ids).await,
    }
    .map_err(|error| {
        tracing::error!(%error, "marking notifications read failed");
        ApiError::internal(&request_id)
    })?;

    let unread_count = notification::unread_count(&mut scoped, actor)
        .await
        .map_err(|error| {
            tracing::error!(%error, "counting unread notifications failed");
            ApiError::internal(&request_id)
        })?;
    unit::commit(tx, &request_id).await?;

    Ok((
        StatusCode::OK,
        axum::Json(MarkReadResult {
            marked,
            unread_count,
        }),
    )
        .into_response())
}

/// What a mark-read request asked for, once it is known to be coherent.
enum Target {
    All,
    Ids(Vec<Uuid>),
}

impl Target {
    fn of(body: &MarkRead, request_id: &str) -> Result<Self, ApiError> {
        match (&body.ids, body.all) {
            (Some(_), Some(true)) => Err(ApiError::bad_request(
                codes::MALFORMED_BODY,
                "Send either `ids` or `all`, not both",
                request_id,
            )),
            (Some(ids), _) if ids.len() > MAX_IDS => Err(ApiError::bad_request(
                codes::OUT_OF_RANGE,
                format!("at most {MAX_IDS} ids per request"),
                request_id,
            )),
            (Some(ids), _) => Ok(Self::Ids(ids.clone())),
            (None, Some(true)) => Ok(Self::All),
            // Neither, or `all: false`. An empty request that silently marked
            // everything read would be the worst possible default.
            (None, _) => Err(ApiError::bad_request(
                codes::MISSING_FIELD,
                "Send `ids` with the notifications to mark, or `all: true`",
                request_id,
            )),
        }
    }
}

/// The cursor for the inbox: the unread flag, the timestamp, and the id.
///
/// The unread flag is in the key because it is the leading sort column
/// (migration 0024). A cursor carrying only the timestamp would jump back into
/// the unread section as soon as a page crossed into the read one.
fn encode_cursor(row: &NotificationRow) -> String {
    Cursor::new(
        vec![
            row.read_at.is_none().to_string(),
            wire::timestamp(row.created_at),
        ],
        row.id,
    )
    .encode()
}

fn page_size(params: &HashMap<String, String>, request_id: &str) -> Result<u32, ApiError> {
    wire::limit(
        params
            .get("limit")
            .map(|raw| {
                raw.parse::<u32>().map_err(|_| {
                    ApiError::bad_request(
                        codes::PAGE_TOO_LARGE,
                        "limit must be a number",
                        request_id,
                    )
                })
            })
            .transpose()?,
        request_id,
    )
}

fn inbox_cursor(
    params: &HashMap<String, String>,
    request_id: &str,
) -> Result<Option<InboxCursor>, ApiError> {
    let malformed =
        || ApiError::bad_request(codes::BAD_CURSOR, "Malformed pagination cursor", request_id);
    let Some(cursor) = wire::cursor(params.get("cursor").map(String::as_str), request_id)? else {
        return Ok(None);
    };
    let unread: bool = cursor
        .keys
        .first()
        .ok_or_else(malformed)?
        .parse()
        .map_err(|_| malformed())?;
    let created_at = cursor
        .keys
        .get(1)
        .ok_or_else(malformed)
        .and_then(|raw| OffsetDateTime::parse(raw, &Rfc3339).map_err(|_| malformed()))?;
    Ok(Some((unread, created_at, cursor.id)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_mark_read_request_must_say_what_to_mark() {
        // An empty body that marked everything read is the worst possible
        // default: it is irreversible and it is exactly what a buggy client
        // sends.
        let empty: MarkRead = serde_json::from_str("{}").expect("valid");
        assert_eq!(
            Target::of(&empty, "r").err().map(|e| e.code()),
            Some(codes::MISSING_FIELD)
        );
        let refused: MarkRead = serde_json::from_str(r#"{"all":false}"#).expect("valid");
        assert!(Target::of(&refused, "r").is_err());
    }

    #[test]
    fn both_fields_at_once_is_refused_rather_than_resolved() {
        let both: MarkRead =
            serde_json::from_str(r#"{"ids":["018f2c9e-0000-7000-8000-000000000001"],"all":true}"#)
                .expect("valid");
        assert_eq!(
            Target::of(&both, "r").err().map(|e| e.code()),
            Some(codes::MALFORMED_BODY)
        );
    }

    #[test]
    fn the_id_list_is_bounded() {
        // docs/21: every input bounded. An unbounded list is an unbounded
        // UPDATE.
        let ids: Vec<String> = (0..=MAX_IDS).map(|_| Uuid::now_v7().to_string()).collect();
        let body: MarkRead =
            serde_json::from_str(&serde_json::json!({ "ids": ids }).to_string()).expect("valid");
        assert_eq!(
            Target::of(&body, "r").err().map(|e| e.code()),
            Some(codes::OUT_OF_RANGE)
        );
    }

    #[test]
    fn an_unknown_field_does_not_deserialize() {
        // docs/05: unknown request fields are rejected.
        assert!(serde_json::from_str::<MarkRead>(r#"{"al":true}"#).is_err());
    }

    #[test]
    fn the_cursor_round_trips_through_its_three_keys() {
        // The unread flag is part of the key. Dropping it makes a page that
        // crosses from unread into read resume in the wrong section — which
        // shows up as notifications repeating, not as an error.
        let row = NotificationRow {
            id: Uuid::now_v7(),
            user_id: Uuid::now_v7(),
            event_type: "task.assigned".to_owned(),
            reason: "ASSIGNED".to_owned(),
            aggregate_id: Some(Uuid::now_v7()),
            payload: serde_json::Value::Null,
            created_at: OffsetDateTime::UNIX_EPOCH,
            read_at: None,
        };
        let encoded = encode_cursor(&row);
        let params = HashMap::from([("cursor".to_owned(), encoded)]);
        let decoded = inbox_cursor(&params, "r").expect("valid").expect("present");
        assert!(decoded.0, "an unread row must resume as unread");
        assert_eq!(decoded.1, OffsetDateTime::UNIX_EPOCH);
        assert_eq!(decoded.2, row.id);
    }

    #[test]
    fn a_read_row_encodes_the_other_side_of_the_boundary() {
        let row = NotificationRow {
            id: Uuid::now_v7(),
            user_id: Uuid::now_v7(),
            event_type: "task.assigned".to_owned(),
            reason: "ASSIGNED".to_owned(),
            aggregate_id: None,
            payload: serde_json::Value::Null,
            created_at: OffsetDateTime::UNIX_EPOCH,
            read_at: Some(OffsetDateTime::UNIX_EPOCH),
        };
        let params = HashMap::from([("cursor".to_owned(), encode_cursor(&row))]);
        let decoded = inbox_cursor(&params, "r").expect("valid").expect("present");
        assert!(!decoded.0);
    }

    #[test]
    fn a_garbage_cursor_is_a_400_and_not_a_panic() {
        for raw in ["!!!", "", "eyJrIjpbXSwiaWQiOiIwMTkyIn0"] {
            let params = HashMap::from([("cursor".to_owned(), raw.to_owned())]);
            let outcome = inbox_cursor(&params, "r");
            assert!(
                outcome.is_err() || outcome.expect("checked").is_none(),
                "{raw:?} was accepted"
            );
        }
    }
}
