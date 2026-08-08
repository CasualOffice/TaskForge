//! Two questions the API asks the database at startup and on every readiness
//! probe.
//!
//! They live here rather than in `casual-task-api` because `docs/19`
//! §Boundary invariants puts **all** SQL in this crate, and `casual-task-lint`
//! makes that a build failure rather than a review comment. The queries are
//! one line each; the rule is not worth a hole.

/// Can the database be reached?
///
/// The cheapest possible query on purpose. A readiness probe that ran a real
/// query would report unready whenever that query was slow, which conflates
/// "cannot serve" with "is busy" — and taking an instance out of rotation for
/// being busy is how a load spike becomes an outage.
///
/// # Errors
///
/// Any database error, including the acquire timeout — which is the case that
/// matters, because it is what a saturated pool looks like from here.
pub async fn ping(pool: &sqlx::PgPool) -> Result<(), sqlx::Error> {
    sqlx::query_scalar::<_, i32>("SELECT 1")
        .fetch_one(pool)
        .await
        .map(|_| ())
}

/// Is the connected role a superuser?
///
/// A superuser bypasses **every** row-level security policy unconditionally and
/// is unaffected by the `REVOKE`s that make audit history append-only. Connected
/// as one the application still works perfectly: every request succeeds, every
/// test passes, and tenant isolation and audit immutability are both silently
/// inert. There is no symptom until one customer sees another's tasks.
///
/// `docs/48` therefore requires the API to refuse to start. This is the
/// question it asks.
///
/// # Errors
///
/// Any database error.
pub async fn is_superuser(pool: &sqlx::PgPool) -> Result<bool, sqlx::Error> {
    sqlx::query_scalar("SELECT current_setting('is_superuser') = 'on'")
        .fetch_one(pool)
        .await
}
