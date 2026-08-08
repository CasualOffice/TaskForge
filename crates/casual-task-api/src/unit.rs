//! The transaction, the authorization answer, and the idempotency claim —
//! the three things every mutating handler does identically.
//!
//! Collected here rather than repeated per handler because each of them has a
//! rule attached that is easy to get subtly wrong once and impossible to notice
//! afterwards: a transaction that is not scoped sees nothing rather than
//! erroring (`docs/32`), a denial reported as `403` on an invisible resource
//! leaks its existence (`docs/04`), and an idempotency claim taken outside the
//! caller's transaction does not serialize retries (`docs/24`).

use std::collections::HashMap;

use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use casual_task_app::{Decision, DenyReason};
use casual_task_persistence::{Scoped, idempotency};
use sha2::{Digest, Sha256};
use sqlx::{Postgres, Transaction};

use crate::error::{ApiError, codes};
use crate::middleware::WorkspaceMember;
use crate::server::AppState;

/// The header carrying a create's idempotency key (`docs/05` §Idempotency).
pub const IDEMPOTENCY_HEADER: &str = "idempotency-key";

/// The longest key accepted. `docs/21` bounds every input; a UUIDv7 in its
/// canonical form is 36 characters, and this leaves room for a client that
/// prefixes its own.
const MAX_KEY: usize = 200;

/// Open the transaction a request commits in.
///
/// # Errors
///
/// `503` when the pool cannot supply a connection — D-039: a bounded pool with
/// a short acquire timeout sheds load rather than queueing behind it.
pub async fn begin(
    state: &AppState,
    request_id: &str,
) -> Result<Transaction<'static, Postgres>, ApiError> {
    state.pool.begin().await.map_err(|error| {
        tracing::warn!(%error, "could not begin a transaction");
        ApiError::unavailable(request_id, 5)
    })
}

/// Apply the request's tenant scope.
///
/// # Errors
///
/// `500`. A failure here must abort rather than continue: an unscoped
/// transaction sees **no rows** rather than erroring (`docs/32`), so carrying
/// on would look like an empty workspace instead of a fault.
pub async fn scope<'t>(
    tx: &'t mut Transaction<'static, Postgres>,
    member: &WorkspaceMember,
    request_id: &str,
) -> Result<Scoped<'t>, ApiError> {
    Scoped::apply(tx, &member.context.scope())
        .await
        .map_err(|error| {
            tracing::error!(%error, "applying the tenant scope failed");
            ApiError::internal(request_id)
        })
}

/// Commit, or report the failure as an internal error.
///
/// # Errors
///
/// `500`.
pub async fn commit(tx: Transaction<'static, Postgres>, request_id: &str) -> Result<(), ApiError> {
    tx.commit().await.map_err(|error| {
        tracing::error!(%error, "commit failed");
        ApiError::internal(request_id)
    })
}

/// Turn a resolver decision into either "carry on" or the right `403`.
///
/// **Only call this for a resource the actor can already see.** `docs/04`
/// requires absent and invisible to be indistinguishable, so visibility is
/// answered with a `404` first and this is reached only afterwards.
///
/// # Errors
///
/// `403 TF-AZN-0001` when no grant carried the permission, `403 TF-AZN-0002`
/// when one did but its constraints were not satisfied. `docs/20` keeps them
/// distinct because they lead a user to different actions.
pub fn authorized(decision: Decision, request_id: &str) -> Result<(), ApiError> {
    match decision {
        Decision::Allow => Ok(()),
        Decision::Deny(DenyReason::NoGrant) => Err(ApiError::denied(codes::NO_GRANT, request_id)),
        Decision::Deny(DenyReason::ConstraintUnsatisfied) => {
            Err(ApiError::denied(codes::CONSTRAINT_UNSATISFIED, request_id))
        }
    }
}

/// The `Idempotency-Key` header. `docs/05`: **required** on `POST` creates.
///
/// # Errors
///
/// `400 TF-IDM-0003` when absent, or when it is empty or over-long.
pub fn idempotency_key(headers: &HeaderMap, request_id: &str) -> Result<String, ApiError> {
    let required = || {
        ApiError::bad_request(
            codes::IDEMPOTENCY_REQUIRED,
            "Idempotency-Key is required on creates: a timeout that actually \
             succeeded would otherwise produce a duplicate nobody can detect",
            request_id,
        )
    };
    let value = headers
        .get(IDEMPOTENCY_HEADER)
        .and_then(|v| v.to_str().ok())
        .map(str::trim)
        .filter(|v| !v.is_empty() && v.len() <= MAX_KEY)
        .ok_or_else(required)?;
    Ok(value.to_owned())
}

/// A stable digest of the parts that define a create.
///
/// `docs/24`: this is what catches "a key generated once and reused for a
/// different task". Hashing the meaningful fields rather than the raw bytes
/// means a client that reorders its JSON keys on retry still replays instead of
/// being told its body changed.
#[must_use]
pub fn hash(parts: &[&[u8]]) -> String {
    let mut hasher = Sha256::new();
    for part in parts {
        // Length-prefixed, so ("ab","c") and ("a","bc") do not collide.
        hasher.update(u32::try_from(part.len()).unwrap_or(u32::MAX).to_be_bytes());
        hasher.update(part);
    }
    hasher
        .finalize()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

/// Claim the key, and return the stored response when this is a replay.
///
/// `Ok(None)` means the caller should perform the create.
///
/// # Errors
///
/// `422 TF-IDM-0002` when the key was used with a different body, `409
/// TF-IDM-0001` when a request with this key has not finished.
pub async fn replay(
    scoped: &mut Scoped<'_>,
    actor: uuid::Uuid,
    key: &str,
    request_hash: &str,
    request_id: &str,
) -> Result<Option<Response>, ApiError> {
    let claim = idempotency::claim(scoped, actor, key, request_hash)
        .await
        .map_err(|error| {
            tracing::error!(%error, "claiming the idempotency key failed");
            ApiError::internal(request_id)
        })?;

    match claim {
        idempotency::Claim::Fresh => Ok(None),
        idempotency::Claim::Replay {
            status_code,
            response,
        } => {
            let status = u16::try_from(status_code)
                .ok()
                .and_then(|s| StatusCode::from_u16(s).ok())
                .unwrap_or(StatusCode::OK);
            Ok(Some((status, axum::Json(response)).into_response()))
        }
        idempotency::Claim::InProgress => Err(ApiError::conflict(
            codes::IDEMPOTENCY_IN_PROGRESS,
            "A request with this Idempotency-Key is still in progress",
            request_id,
        )),
        idempotency::Claim::BodyChanged => Err(ApiError::unprocessable(
            codes::IDEMPOTENCY_BODY_CHANGED,
            "This Idempotency-Key was already used with a different body",
            request_id,
        )),
    }
}

/// Refuse a query parameter this endpoint does not know.
///
/// `docs/05` rejects unknown request *fields*; the same argument applies to
/// query parameters, and for the same reason — `?limt=10` silently returning
/// the default page is a client bug that looks like a server bug.
///
/// # Errors
///
/// `400 TF-VAL-0002`, naming every unknown parameter at once (`docs/05`:
/// "`details` ... returns **all** violations at once, never the first one").
pub fn reject_unknown(
    params: &HashMap<String, String>,
    known: &[&str],
    request_id: &str,
) -> Result<(), ApiError> {
    let mut unknown: Vec<&str> = params
        .keys()
        .map(String::as_str)
        .filter(|k| !known.contains(k))
        .collect();
    if unknown.is_empty() {
        return Ok(());
    }
    unknown.sort_unstable();
    Err(
        ApiError::bad_request(codes::UNKNOWN_FIELD, "Unknown query parameter", request_id)
            .with_details(serde_json::json!({ "unknown": unknown, "known": known })),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::HeaderValue;

    fn headers(value: Option<&str>) -> HeaderMap {
        let mut headers = HeaderMap::new();
        if let Some(value) = value {
            headers.insert(
                IDEMPOTENCY_HEADER,
                HeaderValue::from_str(value).expect("ok"),
            );
        }
        headers
    }

    #[test]
    fn the_two_denial_reasons_stay_distinct() {
        // docs/20: "you were never given this" and "you have it, but not for
        // this object" lead a user to different actions, and
        // /permissions/explain returns the difference.
        assert_eq!(
            authorized(Decision::Deny(DenyReason::NoGrant), "r")
                .err()
                .map(|e| e.code()),
            Some(codes::NO_GRANT)
        );
        assert_eq!(
            authorized(Decision::Deny(DenyReason::ConstraintUnsatisfied), "r")
                .err()
                .map(|e| e.code()),
            Some(codes::CONSTRAINT_UNSATISFIED)
        );
        assert!(authorized(Decision::Allow, "r").is_ok());
    }

    #[test]
    fn a_denial_is_403_and_never_404() {
        // The other direction of docs/04's rule: once the actor CAN see the
        // resource, hiding the refusal behind a 404 would be unhelpful rather
        // than safe.
        let error = authorized(Decision::Deny(DenyReason::NoGrant), "r").expect_err("denied");
        assert_eq!(error.status(), StatusCode::FORBIDDEN);
    }

    #[test]
    fn an_absent_idempotency_key_is_refused() {
        // docs/05: "Required on POST creates."
        assert_eq!(
            idempotency_key(&headers(None), "r").err().map(|e| e.code()),
            Some(codes::IDEMPOTENCY_REQUIRED)
        );
        for bad in ["", "   "] {
            assert!(
                idempotency_key(&headers(Some(bad)), "r").is_err(),
                "{bad:?}"
            );
        }
        assert!(idempotency_key(&headers(Some(&"k".repeat(201))), "r").is_err());
        assert_eq!(
            idempotency_key(&headers(Some(" abc ")), "r").ok(),
            Some("abc".to_owned())
        );
    }

    #[test]
    fn the_request_hash_separates_fields_that_would_otherwise_run_together() {
        // Without length prefixes, ("ab","c") and ("a","bc") hash the same, so
        // two genuinely different creates would replay each other's response.
        assert_ne!(
            hash(&[b"ab", b"c"]),
            hash(&[b"a", b"bc"]),
            "field boundaries are not part of the digest"
        );
        assert_eq!(hash(&[b"a", b"b"]), hash(&[b"a", b"b"]));
    }

    #[test]
    fn an_unknown_query_parameter_names_itself() {
        let params = HashMap::from([
            ("limt".to_owned(), "10".to_owned()),
            ("cursor".to_owned(), "x".to_owned()),
        ]);
        let error = reject_unknown(&params, &["limit", "cursor"], "r").expect_err("refused");
        assert_eq!(error.code(), codes::UNKNOWN_FIELD);
        assert!(reject_unknown(&HashMap::new(), &["limit"], "r").is_ok());
    }
}
