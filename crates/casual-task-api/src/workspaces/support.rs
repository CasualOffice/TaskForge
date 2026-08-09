//! Helpers shared by the workspace handlers.

use axum::extract::Query;
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::{IntoResponse, Response};
use casual_task_model::ActorType;
use casual_task_observability::labels::LabelSet;
use casual_task_observability::metrics::AUTHZ_EPOCH_BUMPS_TOTAL;
use casual_task_persistence::workspace as repo;
use casual_task_persistence::{Provenance, Scoped};
use serde::Serialize;
use time::OffsetDateTime;
use time::format_description::well_known::Rfc3339;
use uuid::Uuid;

use super::*;
use crate::error::{ApiError, codes};
use crate::middleware::{Authenticated, WorkspaceMember};
use crate::server::AppState;

/// Refuse a non-person actor on the two pre-workspace routes.
///
/// A bearer token is "scoped to one workspace" (`docs/40`), so using one to
/// create a *different* workspace, or to enumerate the workspaces of the person
/// it was issued against, is outside the contract the token was issued under.
/// 403 rather than 404: the endpoint is not hidden, the credential is simply
/// not the right kind.
pub(crate) fn only_a_person(actor: &Authenticated, request_id: &str) -> Result<(), ApiError> {
    if matches!(actor.actor_type, ActorType::User) {
        Ok(())
    } else {
        Err(ApiError::forbidden(
            codes::WRONG_CREDENTIAL_TYPE,
            request_id,
        ))
    }
}

pub(crate) async fn begin(
    state: &AppState,
    request_id: &str,
) -> Result<sqlx::Transaction<'static, sqlx::Postgres>, ApiError> {
    state
        .pool
        .begin()
        .await
        .map_err(|_| ApiError::unavailable(request_id, 5))
}

pub(crate) async fn acquire(
    state: &AppState,
    request_id: &str,
) -> Result<sqlx::pool::PoolConnection<sqlx::Postgres>, ApiError> {
    state
        .pool
        .acquire()
        .await
        .map_err(|_| ApiError::unavailable(request_id, 5))
}

pub(crate) async fn scope_of<'t>(
    tx: &'t mut sqlx::Transaction<'static, sqlx::Postgres>,
    member: &WorkspaceMember,
    request_id: &str,
) -> Result<Scoped<'t>, ApiError> {
    Scoped::apply(tx, &member.context.scope())
        .await
        .map_err(|error| internal(&error, "applying the tenant scope", request_id))
}

pub(crate) async fn commit(
    tx: sqlx::Transaction<'static, sqlx::Postgres>,
    request_id: &str,
) -> Result<(), ApiError> {
    tx.commit()
        .await
        .map_err(|error| internal(&error, "committing", request_id))
}

/// Bump `authz_epoch` and count it (`docs/46` §Domain metrics).
pub(crate) async fn bump_epoch(
    state: &AppState,
    scoped: &mut Scoped<'_>,
    request_id: &str,
) -> Result<(), ApiError> {
    repo::bump_authz_epoch(scoped)
        .await
        .map_err(|error| internal(&error, "bumping authz_epoch", request_id))?;
    // A metric failure must never fail a membership change.
    let _ = state.metrics.increment(
        AUTHZ_EPOCH_BUMPS_TOTAL,
        &LabelSet::for_metric(AUTHZ_EPOCH_BUMPS_TOTAL),
        1,
    );
    Ok(())
}

pub(crate) fn provenance(
    actor: &Authenticated,
    request_id: &str,
    headers: &HeaderMap,
) -> Provenance {
    Provenance {
        actor: Some(actor.actor_id),
        actor_type: actor.actor_type,
        request_id: Uuid::parse_str(request_id)
            .ok()
            .map(casual_task_model::RequestId::from_uuid),
        correlation_id: None,
        ip: crate::auth::client_ip(headers),
        user_agent: headers
            .get(header::USER_AGENT)
            .and_then(|v| v.to_str().ok())
            .map(ToOwned::to_owned),
    }
}

pub(crate) fn provenance_member(
    member: &WorkspaceMember,
    request_id: &str,
    headers: &HeaderMap,
) -> Provenance {
    Provenance {
        actor: Some(member.context.actor_id()),
        actor_type: member.context.actor_type(),
        request_id: Uuid::parse_str(request_id)
            .ok()
            .map(casual_task_model::RequestId::from_uuid),
        correlation_id: None,
        ip: crate::auth::client_ip(headers),
        user_agent: headers
            .get(header::USER_AGENT)
            .and_then(|v| v.to_str().ok())
            .map(ToOwned::to_owned),
    }
}

/// `docs/24`: the version is exposed as an `ETag` and required back as
/// `If-Match`.
pub(crate) fn with_etag(status: StatusCode, version: i64, body: WorkspaceBody) -> Response {
    let mut response = (status, axum::Json(body)).into_response();
    if let Ok(value) = format!("\"{version}\"").parse() {
        response.headers_mut().insert(header::ETAG, value);
    }
    response
}

/// The version from `If-Match`, or the documented refusal.
pub(crate) fn if_match(headers: &HeaderMap, request_id: &str) -> Result<i64, ApiError> {
    let raw = headers
        .get(header::IF_MATCH)
        .and_then(|value| value.to_str().ok())
        .ok_or_else(|| ApiError::precondition_required(request_id))?;

    raw.trim()
        .trim_start_matches("W/")
        .trim_matches('"')
        .parse::<i64>()
        .map_err(|_| {
            ApiError::bad_request(
                codes::IF_MATCH_MALFORMED,
                "If-Match is not an ETag this server issued",
                request_id,
            )
        })
}

/// Validate the page request, returning `(limit, after_id)`.
pub(crate) fn page_request(
    paging: Result<Query<Paging>, axum::extract::rejection::QueryRejection>,
    request_id: &str,
) -> Result<(u32, Option<Uuid>), ApiError> {
    let paging = query(paging, request_id)?;
    let limit = limit_of(&paging, request_id)?;
    let after = match paging.cursor.as_deref() {
        None => None,
        Some(raw) => Some(decode_cursor(raw, request_id)?.id),
    };
    Ok((limit, after))
}

/// The same, for a list keyed by a text column rather than by id.
pub(crate) fn page_request_text(
    paging: Result<Query<Paging>, axum::extract::rejection::QueryRejection>,
    request_id: &str,
) -> Result<(u32, Option<String>), ApiError> {
    let paging = query(paging, request_id)?;
    let limit = limit_of(&paging, request_id)?;
    let after = match paging.cursor.as_deref() {
        None => None,
        Some(raw) => decode_cursor(raw, request_id)?.keys.into_iter().next(),
    };
    Ok((limit, after))
}

pub(crate) fn query(
    paging: Result<Query<Paging>, axum::extract::rejection::QueryRejection>,
    request_id: &str,
) -> Result<Paging, ApiError> {
    // A rejection here is an unknown or unparseable query parameter, which
    // `docs/05` makes a 400 rather than something silently ignored.
    paging.map(|Query(paging)| paging).map_err(|_| {
        ApiError::bad_request(
            codes::UNKNOWN_FIELD,
            "Unknown or malformed query parameter",
            request_id,
        )
    })
}

pub(crate) fn limit_of(paging: &Paging, request_id: &str) -> Result<u32, ApiError> {
    match paging.limit {
        None => Ok(DEFAULT_LIMIT),
        // Clamping silently would return a page the client did not ask for and
        // has no way to notice; `docs/20` has a code for it (`TF-QRY-0007`),
        // which is a decision that it is an error rather than a courtesy.
        Some(limit) if limit == 0 || limit > MAX_LIMIT => Err(ApiError::bad_request(
            codes::OUT_OF_RANGE,
            "limit must be between 1 and 100",
            request_id,
        )),
        Some(limit) => Ok(limit),
    }
}

pub(crate) fn decode_cursor(
    raw: &str,
    request_id: &str,
) -> Result<casual_task_model::Cursor, ApiError> {
    casual_task_model::Cursor::decode(raw).map_err(|_| {
        ApiError::bad_request(
            codes::MALFORMED_BODY,
            "Malformed pagination cursor",
            request_id,
        )
    })
}

pub(crate) fn cursor_for(id: Uuid) -> String {
    casual_task_model::Cursor::new(Vec::new(), id).encode()
}

pub(crate) fn encode_cursor(key: &str, id: Uuid) -> String {
    casual_task_model::Cursor::new(vec![key.to_owned()], id).encode()
}

/// The id immediately below `id`, so a keyset walk starting `> after` includes
/// `id` itself.
///
/// Used to read one specific row back through the same paged query rather than
/// adding a second statement that could drift from it.
pub(crate) fn previous_uuid(id: Uuid) -> Option<Uuid> {
    let n = id.as_u128();
    n.checked_sub(1).map(Uuid::from_u128)
}

/// Drop the probe row fetched to detect a next page, reporting whether it was
/// there.
pub(crate) fn truncate<T>(rows: &mut Vec<T>, limit: u32) -> bool {
    let limit = limit as usize;
    if rows.len() > limit {
        rows.truncate(limit);
        true
    } else {
        false
    }
}

pub(crate) fn page<T: Serialize>(data: Vec<T>, has_more: bool, next: Option<String>) -> Response {
    (
        StatusCode::OK,
        axum::Json(PageBody {
            data,
            page: PageInfo {
                next_cursor: if has_more { next } else { None },
                has_more,
            },
        }),
    )
        .into_response()
}

pub(crate) fn valid_name<'a>(name: &'a str, request_id: &str) -> Result<&'a str, ApiError> {
    let trimmed = name.trim();
    if trimmed.is_empty() || trimmed.chars().count() > MAX_NAME {
        return Err(ApiError::bad_request(
            codes::OUT_OF_RANGE,
            "name must be between 1 and 200 characters",
            request_id,
        ));
    }
    Ok(trimmed)
}

/// A slug is URL-visible, so its character set is bounded rather than trusted.
pub(crate) fn valid_slug<'a>(slug: &'a str, request_id: &str) -> Result<&'a str, ApiError> {
    let ok = !slug.is_empty()
        && slug.len() <= MAX_SLUG
        && slug.starts_with(|c: char| c.is_ascii_lowercase() || c.is_ascii_digit())
        && slug
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-');
    if ok {
        Ok(slug)
    } else {
        Err(ApiError::bad_request(
            codes::OUT_OF_RANGE,
            "slug must be 1-64 characters of a-z, 0-9 and -, starting with a letter or digit",
            request_id,
        ))
    }
}

pub(crate) fn workspace_body(record: &repo::WorkspaceRecord) -> WorkspaceBody {
    WorkspaceBody {
        id: record.id,
        name: record.name.clone(),
        slug: record.slug.clone(),
        created_at: rfc3339(record.created_at),
    }
}

pub(crate) fn member_body(record: &repo::MemberRecord) -> MemberBody {
    MemberBody {
        user_id: record.user_id,
        display_name: record.display_name.clone(),
        email: record.email.clone(),
        member_type: record.member_type.clone(),
        joined_at: rfc3339(record.joined_at),
    }
}

pub(crate) fn team_member_body(record: &repo::TeamMemberRecord) -> TeamMemberBody {
    TeamMemberBody {
        user_id: record.user_id,
        display_name: record.display_name.clone(),
        email: record.email.clone(),
        member_type: record.member_type.clone(),
    }
}

pub(crate) fn team_body(record: &repo::TeamRecord) -> TeamBody {
    TeamBody {
        id: record.id,
        name: record.name.clone(),
        created_at: rfc3339(record.created_at),
    }
}

/// `docs/05` §Conventions: RFC 3339, always UTC, always `Z`.
pub(crate) fn rfc3339(at: OffsetDateTime) -> String {
    at.to_offset(time::UtcOffset::UTC)
        .format(&Rfc3339)
        .unwrap_or_default()
}

pub(crate) fn unique_violation(error: &sqlx::Error) -> bool {
    sqlstate(error).as_deref() == Some("23505")
}

pub(crate) fn foreign_key_violation(error: &sqlx::Error) -> bool {
    sqlstate(error).as_deref() == Some("23503")
}

pub(crate) fn sqlstate(error: &sqlx::Error) -> Option<String> {
    error
        .as_database_error()
        .and_then(|e| e.code())
        .map(|c| c.into_owned())
}

/// Log the cause, return the opaque envelope.
///
/// The detail belongs in the log correlated by `request_id`; in the response it
/// is reconnaissance (`docs/05`).
pub(crate) fn internal(error: &sqlx::Error, doing: &str, request_id: &str) -> ApiError {
    tracing::error!(%error, doing, "workspace request failed");
    ApiError::internal(request_id)
}

pub(crate) fn internal_message(doing: &str, request_id: &str) -> ApiError {
    tracing::error!(doing, "workspace request failed");
    ApiError::internal(request_id)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_slug_cannot_carry_anything_a_url_would_have_to_escape() {
        for bad in [
            "",
            "-leading",
            "Upper",
            "with space",
            "with/slash",
            "with.dot",
            "..",
            "a", // fine — kept below as the positive case
        ] {
            let result = valid_slug(bad, "r");
            if bad == "a" {
                assert!(result.is_ok());
            } else {
                assert!(result.is_err(), "accepted slug {bad:?}");
            }
        }
        assert!(valid_slug(&"a".repeat(MAX_SLUG), "r").is_ok());
        assert!(valid_slug(&"a".repeat(MAX_SLUG + 1), "r").is_err());
    }

    #[test]
    fn a_name_is_bounded_and_not_blank() {
        assert!(valid_name("  ", "r").is_err());
        assert!(valid_name(&"x".repeat(MAX_NAME + 1), "r").is_err());
        assert_eq!(valid_name("  Acme  ", "r").expect("valid"), "Acme");
    }

    #[test]
    fn if_match_is_required_and_then_parsed() {
        let mut headers = HeaderMap::new();
        assert_eq!(
            if_match(&headers, "r").expect_err("required").status(),
            StatusCode::PRECONDITION_REQUIRED
        );

        headers.insert(header::IF_MATCH, "\"7\"".parse().expect("valid"));
        assert_eq!(if_match(&headers, "r").expect("parsed"), 7);

        // Weak validators are what a caching proxy may rewrite an ETag into.
        headers.insert(header::IF_MATCH, "W/\"7\"".parse().expect("valid"));
        assert_eq!(if_match(&headers, "r").expect("parsed"), 7);

        headers.insert(header::IF_MATCH, "\"nonsense\"".parse().expect("valid"));
        assert_eq!(
            if_match(&headers, "r").expect_err("malformed").status(),
            StatusCode::BAD_REQUEST
        );
    }

    #[test]
    fn the_page_limit_is_bounded_at_the_documented_values() {
        // docs/05: default 50, max 100. A client asking for 10_000 is refused
        // rather than quietly served 100, because a silently clamped page is
        // indistinguishable to the client from a short last page.
        let with = |limit| Paging {
            limit,
            cursor: None,
        };
        assert_eq!(limit_of(&with(None), "r").expect("default"), DEFAULT_LIMIT);
        assert_eq!(limit_of(&with(Some(100)), "r").expect("max"), MAX_LIMIT);
        assert!(limit_of(&with(Some(0)), "r").is_err());
        assert!(limit_of(&with(Some(101)), "r").is_err());
    }

    #[test]
    fn the_probe_row_is_dropped_and_reported() {
        let mut rows = vec![1, 2, 3];
        assert!(truncate(&mut rows, 2));
        assert_eq!(rows, vec![1, 2]);

        let mut rows = vec![1, 2];
        assert!(!truncate(&mut rows, 2));
        assert_eq!(rows, vec![1, 2]);
    }

    #[test]
    fn a_cursor_round_trips_and_is_not_an_offset() {
        let id = Uuid::now_v7();
        let encoded = cursor_for(id);
        assert_eq!(
            casual_task_model::Cursor::decode(&encoded)
                .expect("decodes")
                .id,
            id
        );
        assert!(!encoded.contains('='), "cursors are base64url, unpadded");
    }

    #[test]
    fn timestamps_are_utc_with_a_z() {
        // docs/05 §Conventions: "RFC 3339, always UTC, always Z". An
        // OffsetDateTime carrying +05:30 formats as +05:30 unless converted.
        let at = OffsetDateTime::from_unix_timestamp(1_767_225_600)
            .expect("valid")
            .to_offset(time::UtcOffset::from_hms(5, 30, 0).expect("valid"));
        assert!(rfc3339(at).ends_with('Z'), "{}", rfc3339(at));
    }
}
