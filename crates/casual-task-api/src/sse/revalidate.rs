//! What makes an already-open stream stop being allowed.
//!
//! # The failure this file exists to prevent
//!
//! Revocation that is only true for requests. `docs/40`'s acceptance gate says
//! "a revoked session is rejected on the next request; **an SSE stream held by
//! that session closes**" — and a stream has no next request. Authorized once at
//! connect and never asked again, it keeps delivering events to a credential
//! that was destroyed hours ago. That is not a slow revocation; it is no
//! revocation at all for the one channel that stays open.
//!
//! Split from [`super::endpoint`] because the two change for different reasons:
//! the endpoint changes with `docs/05`'s wire format, and this file changes with
//! the answer to "who is still allowed to be here".
//!
//! # Two questions, deliberately asymmetric in cost
//!
//! **Is the credential still live?** Asked every tick. It is one indexed read,
//! and it is the question `docs/40` names.
//!
//! **Is the authority still sufficient?** Asked only when
//! `workspace.authz_epoch` has moved. `docs/04` defines that counter as bumped
//! by "any grant, role, team membership, or project membership change, in the
//! same transaction as the change", so an unchanged epoch is proof that
//! re-resolving would produce the same answer. This is `docs/05`'s "membership
//! is revalidated on every `authz_epoch` change", implemented literally: the
//! expensive check runs when the cheap one says something moved.
//!
//! # Failing open, and where it does not
//!
//! A tick that cannot reach the database does **not** cancel. A database blip
//! would otherwise drop every open stream in the deployment at once — a
//! self-inflicted outage triggered by a transient fault, and one that arrives
//! precisely when the system is already unwell.
//!
//! It fails **closed** on every answer that is an answer: a 401 cancels, a
//! refused authorization cancels. The distinction is the HTTP status, not a
//! guess: `authenticate` reports a dead credential as `401` and a broken
//! database as `5xx`.

use std::time::Duration;

use axum::http::{HeaderMap, StatusCode};
use casual_task_infra::broadcast::Canceller;
use casual_task_persistence::workspace;
use uuid::Uuid;

use crate::context::Context;
use crate::middleware::WorkspaceMember;
use crate::server::AppState;
use crate::sse::authorize;

/// How long a revoked subscriber may still receive events.
///
/// # The cost, both directions
///
/// This interval **is** the revocation window: a session destroyed just after a
/// tick keeps receiving events for up to 15 seconds. `docs/40` does not put a
/// number on it — it says the stream closes — so the number is chosen here and
/// stated here.
///
/// The other direction is load. Every open stream costs one indexed session read
/// per tick, plus one epoch read: at 1,000 concurrent streams that is roughly
/// 133 queries a second, and at the hub's 10,000-subscriber cap roughly 1,330.
/// That is real, it is the dominant database cost of this feature, and it scales
/// with connections rather than with events.
///
/// Shorter would narrow the window and multiply that load; longer would widen
/// the window a security gate exists to close. Fifteen seconds is half the
/// heartbeat in `docs/05`, so a revoked client is cut off well inside the
/// interval it would otherwise notice nothing in.
///
/// The way out of the trade is a shared invalidation signal — PostgreSQL
/// `LISTEN`/`NOTIFY` on revocation and epoch bumps — which removes the polling
/// entirely. That is a design change, not a constant, and it is not built.
pub const INTERVAL: Duration = Duration::from_secs(15);

/// Everything the tick needs to ask its two questions again.
#[allow(missing_debug_implementations)]
pub struct Watch {
    pub state: AppState,
    /// The request's headers, kept for the credential they carry.
    ///
    /// Not the resolved actor: re-checking a *decision* made at connect proves
    /// nothing about now. The credential is re-presented to the same code path
    /// that admitted it.
    pub headers: HeaderMap,
    pub member: WorkspaceMember,
    pub project: Uuid,
    /// The epoch when the stream was authorized.
    pub epoch: i64,
    pub canceller: Canceller,
    pub request_id: String,
    /// How long between checks. [`INTERVAL`] in production.
    ///
    /// A field rather than a constant read inside the loop, so the revocation
    /// behaviour is testable without a test that sleeps for fifteen seconds —
    /// and a test that slow is one that gets marked `#[ignore]` and then stops
    /// running, which for a security gate is the same as deleting it.
    pub interval: Duration,
}

/// Why a stream was ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Ended {
    /// The credential is gone: revoked, expired, or logged out.
    CredentialRevoked,
    /// The credential is fine; the authority behind it no longer permits this
    /// project.
    AuthorityWithdrawn,
}

/// Run the tick until the stream ends or the authority does.
///
/// Returns `Some` when it cancelled the stream, `None` when the stream ended on
/// its own — which is the ordinary case and is why this is a task that stops
/// rather than one that runs forever.
pub async fn watch(mut watch: Watch) -> Option<Ended> {
    loop {
        tokio::time::sleep(watch.interval).await;

        // The stream is gone: the client disconnected, or shutdown closed it.
        // Checked every tick so a closed connection cannot leave a task behind
        // polling the database on its behalf — one leaked task per stream is a
        // slow leak that only shows up under the traffic that produced it.
        if !watch.canceller.is_live() {
            return None;
        }

        match check(&mut watch).await {
            Ok(Some(ended)) => {
                tracing::info!(
                    project_id = %watch.project,
                    reason = ?ended,
                    "closing a live stream: it is no longer authorized"
                );
                watch.canceller.cancel();
                return Some(ended);
            }
            Ok(None) => {}
            // Transient. Deliberately not a cancellation — see the module docs.
            Err(error) => {
                tracing::warn!(%error, "a stream revalidation tick failed; keeping the stream");
            }
        }
    }
}

/// One round of both questions.
async fn check(watch: &mut Watch) -> Result<Option<Ended>, sqlx::Error> {
    let mut conn = watch.state.pool.acquire().await?;
    let live = crate::middleware::authenticate(&mut conn, &watch.headers, &watch.request_id).await;
    drop(conn);

    if let Err(error) = live {
        // Only an authentication answer ends the stream. A 500 here is the
        // database being unwell, and dropping every stream in response is the
        // outage this deliberately does not cause.
        return Ok((error.status() == StatusCode::UNAUTHORIZED).then_some(Ended::CredentialRevoked));
    }

    let mut tx = watch.state.pool.begin().await?;
    let scope = watch.member.context.scope();
    let mut scoped = casual_task_persistence::Scoped::apply(&mut tx, &scope).await?;
    let epoch = workspace::authz_epoch(&mut scoped).await?;
    if epoch == watch.epoch {
        // Nothing that could change the answer has happened. docs/04 makes this
        // sound rather than merely likely: the epoch is bumped in the same
        // transaction as any change that would matter.
        tx.rollback().await?;
        return Ok(None);
    }

    // Something moved. Re-resolve through the same path the endpoint used —
    // not a second implementation of it, which is how one door ends up more
    // permissive than the other.
    let refused = match Context::load(
        &watch.state.metrics,
        &mut scoped,
        &watch.member,
        &watch.headers,
        &watch.request_id,
    )
    .await
    {
        Ok(ctx) => {
            match authorize::may_subscribe(&mut scoped, &ctx, watch.project, &watch.request_id)
                .await
            {
                Ok(decision) => decision.is_err(),
                // An error resolving is not a refusal.
                Err(_) => false,
            }
        }
        Err(_) => false,
    };
    tx.rollback().await?;

    if refused {
        return Ok(Some(Ended::AuthorityWithdrawn));
    }
    // Authorized under the new epoch: remember it, so the expensive check is
    // not repeated every tick for the rest of the stream's life.
    watch.epoch = epoch;
    Ok(None)
}

/// Read the epoch a stream is being authorized under, inside the caller's
/// transaction.
///
/// # Errors
///
/// Any database error.
pub async fn current_epoch(
    scoped: &mut casual_task_persistence::Scoped<'_>,
) -> Result<i64, sqlx::Error> {
    workspace::authz_epoch(scoped).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_revocation_window_is_shorter_than_the_heartbeat() {
        // The heartbeat is the interval a client would otherwise notice nothing
        // in (docs/05: 30 s). A revocation window longer than that would mean a
        // revoked client sees a keep-alive before it sees the door close.
        assert!(
            INTERVAL < super::super::HEARTBEAT,
            "the revocation window ({INTERVAL:?}) is not shorter than the heartbeat"
        );
    }

    #[test]
    fn the_window_is_bounded_at_all() {
        // The defect this whole module closes: before it, the window was the
        // lifetime of the connection.
        assert!(INTERVAL <= Duration::from_secs(30));
        assert!(
            INTERVAL >= Duration::from_secs(5),
            "a shorter tick multiplies a per-connection database cost for a \
             window nobody asked to be that tight"
        );
    }
}
