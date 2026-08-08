//! Idempotent creates (`docs/24` §Idempotency for creates).
//!
//! ```text
//! BEGIN
//!   INSERT INTO idempotency_key (...) ON CONFLICT DO NOTHING;
//!   -- 0 rows ⇒ this key was seen before:
//!   --    same request_hash → return the stored response
//!   --    different hash    → 422 TF-IDM-0002
//!   ... perform the create ...
//!   UPDATE idempotency_key SET response = ..., status_code = 201 WHERE ...;
//! COMMIT
//! ```
//!
//! # Why the claim is inside the caller's transaction
//!
//! `docs/24`: "the `ON CONFLICT DO NOTHING` insert inside the transaction
//! serializes them". A second request carrying the same key blocks on the
//! unique index until the first commits, and then reads the response the first
//! wrote — so a retry cannot produce a second task, and cannot see a half-built
//! one either.
//!
//! # Why the body is hashed
//!
//! `docs/24`: `request_hash` "catches the common client bug of generating a key
//! once and reusing it for a different task. Without it, the second task
//! silently returns the first task's response and the user thinks it was
//! created." The hash is computed at the edge, where the bytes are.

use uuid::Uuid;

use crate::scoped::Scoped;

/// What a key turned out to be.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Claim {
    /// Unseen. The caller performs the create and then calls [`record`].
    Fresh,
    /// Seen, completed. Return this response verbatim.
    Replay {
        status_code: i32,
        response: serde_json::Value,
    },
    /// Seen, not finished — `409 TF-IDM-0001`.
    InProgress,
    /// Seen with a different body — `422 TF-IDM-0002`.
    BodyChanged,
}

/// Claim a key, or discover what it already means.
///
/// # Errors
///
/// Any database error.
pub async fn claim(
    scoped: &mut Scoped<'_>,
    actor: Uuid,
    key: &str,
    request_hash: &str,
) -> Result<Claim, sqlx::Error> {
    let workspace = scoped.workspace_id().as_uuid();
    let inserted = sqlx::query(
        "INSERT INTO idempotency_key (workspace_id, actor_id, key, request_hash)
         VALUES ($1,$2,$3,$4)
         ON CONFLICT (workspace_id, actor_id, key) DO NOTHING",
    )
    .bind(workspace)
    .bind(actor)
    .bind(key)
    .bind(request_hash)
    .execute(scoped.conn())
    .await?
    .rows_affected();

    if inserted == 1 {
        return Ok(Claim::Fresh);
    }

    let existing: Option<(String, Option<i32>, Option<serde_json::Value>)> = sqlx::query_as(
        "SELECT request_hash, status_code, response
           FROM idempotency_key
          WHERE workspace_id = $1 AND actor_id = $2 AND key = $3",
    )
    .bind(workspace)
    .bind(actor)
    .bind(key)
    .fetch_optional(scoped.conn())
    .await?;

    // The row was there for the conflict and is gone now: the transaction that
    // held it rolled back between the two statements. Treating that as fresh
    // would race the retry; treating it as in progress asks the client to try
    // again, which is what the situation actually is.
    let Some((stored_hash, status_code, response)) = existing else {
        return Ok(Claim::InProgress);
    };

    if stored_hash != request_hash {
        return Ok(Claim::BodyChanged);
    }
    match (status_code, response) {
        (Some(status_code), Some(response)) => Ok(Claim::Replay {
            status_code,
            response,
        }),
        _ => Ok(Claim::InProgress),
    }
}

/// Store the response a claimed key produced, in the same transaction as the
/// create it describes.
///
/// # Errors
///
/// Any database error.
pub async fn record(
    scoped: &mut Scoped<'_>,
    actor: Uuid,
    key: &str,
    status_code: i32,
    response: &serde_json::Value,
) -> Result<(), sqlx::Error> {
    let workspace = scoped.workspace_id().as_uuid();
    sqlx::query(
        "UPDATE idempotency_key SET response = $4, status_code = $5
          WHERE workspace_id = $1 AND actor_id = $2 AND key = $3",
    )
    .bind(workspace)
    .bind(actor)
    .bind(key)
    .bind(response)
    .bind(status_code)
    .execute(scoped.conn())
    .await?;
    Ok(())
}
