//! `GET /api/v1/stream` — the wire half of live updates (`docs/05`).
//!
//! # The failure this file exists to prevent
//!
//! A stream that ends by accident. Every other endpoint in this crate answers
//! once and forgets the caller; this one keeps sending for hours, so *how it
//! stops* is as much of the contract as how it starts.
//!
//! Three endings are deliberate here, and none of them is "the socket
//! eventually errors": a `SIGTERM` closes it (`Received::Closed`), a client that
//! stops reading is cut off and told why (`Received::Lagged`), and a client that
//! disconnects takes its subscription with it.
//!
//! A fourth is the one `docs/40` names: a revoked session, or an authority that
//! no longer permits this project, ends the stream from outside it
//! ([`super::revalidate`]) and the client is told to re-authenticate rather than
//! left to reconnect with a credential that will only be refused again.

use std::convert::Infallible;
use std::pin::Pin;
use std::task::{Context as TaskContext, Poll};
use std::time::Duration;

use axum::extract::{Query, State};
use axum::http::HeaderMap;
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::response::{IntoResponse, Response};
use casual_task_infra::broadcast::{Broadcast, Received, Subscription, Topic};
use casual_task_observability::labels::LabelSet;
use casual_task_observability::metrics::SSE_CONNECTIONS_ACTIVE;
use casual_task_observability::recorder::Recorder;
use serde::Deserialize;
use uuid::Uuid;

use crate::context::Context;
use crate::error::ApiError;
use crate::middleware::WorkspaceMember;
use crate::server::{AppState, RequestId};
use crate::sse::{authorize, revalidate};
use crate::unit;

/// `docs/05`: "Heartbeat comment every 30 s keeps intermediaries from closing
/// idle streams."
///
/// A proxy that sees nothing on a connection for its idle timeout closes it, and
/// the client then reconnects — which is survivable but produces a reconnect
/// storm on every deployment with a 60-second proxy timeout. The comment costs
/// two bytes and removes the class.
pub const HEARTBEAT: Duration = Duration::from_secs(30);

/// What the client asks for.
#[derive(Debug, Deserialize)]
pub struct StreamQuery {
    /// `docs/05`: `GET /api/v1/stream?project_id=...`.
    ///
    /// Required, and not defaulted to "everything the actor can see". A
    /// wildcard subscription is one refactor away from being the default, and
    /// its blast radius is every event in the tenant.
    pub project_id: Uuid,
}

/// `GET /api/v1/stream?project_id=...`
///
/// # Errors
///
/// `404` when the project is not visible, `403` when the actor may not read
/// every task in it (see [`authorize`]), `500` on a database failure.
pub async fn stream(
    State(state): State<AppState>,
    member: WorkspaceMember,
    headers: HeaderMap,
    Query(query): Query<StreamQuery>,
) -> Result<Response, ApiError> {
    let request_id = RequestId::of_parts(&headers);

    // The authorization happens in a transaction that is committed BEFORE the
    // stream is returned. Holding one open for the life of a stream would pin a
    // database connection per subscriber — the same shape D-038 rejected for the
    // outbox dispatcher, and worse here, because a stream lasts hours.
    let mut tx = unit::begin(&state, &request_id).await?;
    let mut scoped = unit::scope(&mut tx, &member, &request_id).await?;
    let ctx = Context::load(&mut scoped, &member, &headers, &request_id).await?;
    let decision =
        authorize::may_subscribe(&mut scoped, &ctx, query.project_id, &request_id).await?;
    // Read in the SAME transaction as the authorization, so the pair cannot
    // straddle a grant change: an epoch read afterwards could return a value
    // from after a bump the authorization did not see, and the stream would
    // then never re-check that change.
    let epoch = revalidate::current_epoch(&mut scoped)
        .await
        .map_err(|error| {
            tracing::error!(%error, "reading the authorization epoch for a stream failed");
            ApiError::internal(&request_id)
        })?;
    unit::commit(tx, &request_id).await?;

    if let Err(refusal) = decision {
        return Ok(refusal.into_error(&request_id).into_response());
    }

    let topic = Topic::project(ctx.workspace, query.project_id);
    let subscription = state.broadcast.subscribe(topic);
    record_connections(&state.metrics, state.broadcast.subscriber_count());

    // The other half of docs/40's revocation gate. Authorization at connect
    // decides who gets in; this decides who stays, and without it a revoked
    // session keeps receiving events until the client leaves.
    //
    // A detached task rather than work inside the stream: the checks are
    // database reads, and doing them on the polling path would stall event
    // delivery for every subscriber behind the slowest query. It stops itself
    // when the subscription goes away.
    tokio::spawn(revalidate::watch(revalidate::Watch {
        state: state.clone(),
        headers: headers.clone(),
        member: member.clone(),
        project: query.project_id,
        epoch,
        canceller: subscription.canceller(),
        request_id: request_id.clone(),
        interval: revalidate::INTERVAL,
    }));

    tracing::info!(
        project_id = %query.project_id,
        actor = %ctx.actor.as_uuid(),
        "live stream opened"
    );

    let events = EventStream {
        subscription,
        metrics: state.metrics.clone(),
        broadcast: state.broadcast.clone(),
    };

    Ok(Sse::new(events)
        // axum's keep-alive emits an SSE comment on an idle stream, which is
        // exactly docs/05's heartbeat. Written as a dependency of the framework
        // rather than a timer in the loop below because a hand-rolled one has to
        // race the event channel, and losing that race drops an event.
        .keep_alive(KeepAlive::new().interval(HEARTBEAT))
        .into_response())
}

/// The subscription, as something axum can poll.
///
/// # Why the metric lives on `Drop` and not at the end of a loop
///
/// A stream ends for reasons the handler never sees: the client closes the
/// socket, the runtime drops the task on shutdown, a panic unwinds. A gauge
/// decremented on the happy path only drifts up forever, and the first thing an
/// operator does with a gauge they have stopped trusting is stop looking at it.
struct EventStream {
    subscription: Subscription,
    metrics: std::sync::Arc<Recorder>,
    broadcast: std::sync::Arc<dyn Broadcast>,
}

impl Drop for EventStream {
    fn drop(&mut self) {
        // Read after the subscription is gone... except it is not gone yet —
        // `self.subscription` is dropped after this runs. So the count is
        // adjusted by one here rather than re-read, which is the difference
        // between a gauge that settles at zero and one that settles at one.
        let live = self.broadcast.subscriber_count().saturating_sub(1);
        record_connections(&self.metrics, live);
    }
}

impl futures_core::Stream for EventStream {
    type Item = Result<Event, Infallible>;

    fn poll_next(self: Pin<&mut Self>, cx: &mut TaskContext<'_>) -> Poll<Option<Self::Item>> {
        let this = self.get_mut();
        match this.subscription.poll_recv(cx) {
            Poll::Pending => Poll::Pending,
            Poll::Ready(Received::Event(event)) => Poll::Ready(Some(Ok(Event::default()
                // `id:` is what comes back as `Last-Event-ID`, so it is the
                // event's own id and not a counter — a counter would restart at
                // zero on every deploy and silently mean a different position.
                .id(event.id.to_string())
                .event(event.event_type)
                .data(event.data)))),
            // The overflow policy, told to the client rather than performed
            // silently. `docs/05` gives it `Last-Event-ID` to resume from; a
            // stream that just stopped would leave it believing it was current.
            Poll::Ready(Received::Lagged) => Poll::Ready(Some(Ok(Event::default()
                .event("stream.lagged")
                .data(r#"{"reason":"too_slow","action":"reconnect_with_last_event_id"}"#)))),
            // The stream was revoked out from under the client (`revalidate`).
            // Named rather than silent: a plain end-of-stream would have the
            // client reconnect with the same dead credential, get a 401, and
            // have no idea the two were related.
            Poll::Ready(Received::Cancelled) => {
                Poll::Ready(Some(Ok(Event::default().event("stream.revoked").data(
                    r#"{"reason":"credential_or_authority_revoked","action":"reauthenticate"}"#,
                ))))
            }
            // D-041: shutdown closes the stream. The client sees an ordinary
            // end-of-stream and reconnects, instead of a socket vanishing
            // mid-frame and looking like a parse error.
            Poll::Ready(Received::Closed) => Poll::Ready(None),
        }
    }
}

/// Publish `sse_connections_active` (`docs/46`).
fn record_connections(metrics: &Recorder, live: usize) {
    let labels = LabelSet::for_metric(SSE_CONNECTIONS_ACTIVE);
    #[allow(clippy::cast_precision_loss)]
    if let Err(error) = metrics.set(SSE_CONNECTIONS_ACTIVE, &labels, live as f64) {
        // Logged, never propagated: a metric is not worth ending a stream over.
        tracing::error!(%error, "recording the live-stream count");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_heartbeat_is_shorter_than_a_common_proxy_idle_timeout() {
        // nginx and most load balancers default to 60 s. A heartbeat at or above
        // that is not a heartbeat; it is a reconnect schedule.
        assert!(HEARTBEAT < Duration::from_secs(60));
        assert_eq!(HEARTBEAT, Duration::from_secs(30), "docs/05 says 30 s");
    }

    #[test]
    fn the_query_requires_a_project() {
        // A stream with no project would have to mean "everything visible",
        // which is the wildcard subscription Topic deliberately cannot express.
        let missing = Query::<StreamQuery>::try_from_uri(
            &"http://x/api/v1/stream".parse().expect("a valid uri"),
        );
        assert!(
            missing.is_err(),
            "a stream request with no project_id parsed, so it would have to be \
             given a default — and the only available default is 'everything'"
        );
        let present = Query::<StreamQuery>::try_from_uri(
            &"http://x/api/v1/stream?project_id=018f2c00-0000-7000-8000-000000000001"
                .parse()
                .expect("a valid uri"),
        )
        .expect("a well-formed query");
        assert_eq!(
            present.0.project_id.to_string(),
            "018f2c00-0000-7000-8000-000000000001"
        );
    }
}
