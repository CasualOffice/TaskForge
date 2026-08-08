//! The pre-workspace seam: reading a credential before any tenant is known.
//!
//! `docs/40` §The pre-workspace seam (ADR-032). `api_token` carries
//! `workspace_id` and keeps its row-level security policy, but authentication
//! happens **before** any workspace is known — the request that must read the
//! credential row is exactly the one that cannot yet set
//! `taskforge.workspace_id`.
//!
//! Migration 0016 provides two `SECURITY DEFINER` functions, each returning a
//! fixed projection. This module is the only place that calls them.
//!
//! # Why two functions and not one
//!
//! "Find the row" and "read the secret" are separate grants. A future reader of
//! [`lookup_token`] cannot conclude that adding `verifier_hash` to it would be
//! convenient, because the hash already has its own door — and the schema gate
//! fails the build if it ever appears in the first one.

use uuid::Uuid;

use crate::scoped::Scoped;

/// Identifying material for a token. Deliberately **not** the verifier hash.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TokenIdentity {
    pub id: Uuid,
    pub workspace_id: Uuid,
    pub principal_type: String,
    pub principal_id: Uuid,
}

/// Find a live token by its selector, across every tenant.
///
/// Returns `None` for an unknown, revoked, or expired selector — the three are
/// deliberately indistinguishable here, because `docs/40` §Acceptance gates
/// requires authentication responses not to reveal which.
///
/// # Errors
///
/// Any database error.
pub async fn lookup_token(
    conn: &mut sqlx::PgConnection,
    selector: &str,
) -> Result<Option<TokenIdentity>, sqlx::Error> {
    let row: Option<(Uuid, Uuid, String, Uuid)> = sqlx::query_as(
        "SELECT id, workspace_id, principal_type::text, principal_id
           FROM lookup_api_token($1)",
    )
    .bind(selector)
    .fetch_optional(conn)
    .await?;

    Ok(row.map(
        |(id, workspace_id, principal_type, principal_id)| TokenIdentity {
            id,
            workspace_id,
            principal_type,
            principal_id,
        },
    ))
}

/// The stored verifier hash for a live token.
///
/// Separate grant, separate function — see the module docs.
///
/// # Errors
///
/// Any database error.
pub async fn lookup_token_verifier(
    conn: &mut sqlx::PgConnection,
    selector: &str,
) -> Result<Option<String>, sqlx::Error> {
    sqlx::query_scalar("SELECT lookup_api_token_verifier($1)")
        .bind(selector)
        .fetch_optional(conn)
        .await
        .map(Option::flatten)
}

/// Record that a token was used.
///
/// Takes a [`Scoped`] rather than a bare connection: by the time this is
/// called the workspace **is** known — it came from [`lookup_token`] — so the
/// write goes through the ordinary tenant-scoped path rather than through the
/// seam. The seam is for the one read that cannot.
///
/// # Errors
///
/// Any database error.
pub async fn touch_token(scoped: &mut Scoped<'_>, id: Uuid) -> Result<(), sqlx::Error> {
    sqlx::query("UPDATE api_token SET last_used_at = now() WHERE id = $1")
        .bind(id)
        .execute(scoped.conn())
        .await?;
    Ok(())
}
