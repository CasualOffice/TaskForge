//! One subscriber's end of the hub: what it receives, and every way it stops.
//!
//! # The failure this file exists to prevent
//!
//! A stream that keeps delivering after it stopped being allowed to.
//!
//! `docs/40`'s revocation gate is explicit — "a revoked session is rejected on
//! the next request; **an SSE stream held by that session closes**" — and a
//! long-lived stream has no next request to be rejected on. Something has to
//! reach in from outside and end it, which is what [`Canceller`] is.
//!
//! Split from the hub itself because the two change for different reasons: the
//! hub changes when fan-out or backpressure does, and this file changes when the
//! answer to "why did that stream stop?" does.
//!
//! # Every ending is named
//!
//! [`Received`] has a variant per reason, and none of them is "the socket
//! errored". A client that is told *why* can do the right thing —
//! [`Received::Lagged`] means resume from `Last-Event-ID`, [`Received::Cancelled`]
//! means re-authenticate, [`Received::Closed`] means the server is going away
//! and a plain reconnect is right.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

use tokio::sync::mpsc;

use super::LiveEvent;

/// What arrived on a subscription.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Received {
    /// An event to send to the client.
    Event(Box<LiveEvent>),
    /// The subscriber fell behind its queue bound and was dropped. The client
    /// should reconnect with `Last-Event-ID` (`docs/05`).
    Lagged,
    /// The credential or the authority behind this stream stopped being valid,
    /// and something outside the stream ended it. The client should
    /// re-authenticate rather than blindly reconnect.
    Cancelled,
    /// The hub closed — the process is shutting down (`docs/24` §D-041).
    Closed,
}

/// What the hub sends down the event channel.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum Delivery {
    Event(Box<LiveEvent>),
    Lagged,
}

/// Decrements the live-subscription count exactly once, whoever gets there
/// first.
///
/// # Why once, and why not only on `Drop`
///
/// `sse_connections_active` (`docs/46`) must count streams that can still
/// receive. A cancelled subscription cannot — it is over the moment
/// [`Canceller::cancel`] runs — but the task holding it is not dropped until the
/// runtime next polls it, which can be a while on a busy server. Counting it in
/// between makes the gauge read high exactly during a revocation incident, which
/// is the worst possible time for an operator to distrust it.
///
/// So both cancellation and drop release, and the flag makes the second one a
/// no-op. Without the flag the count would go *down twice* for one stream and
/// the gauge would drift below zero — worse than the drift it was fixing.
#[derive(Debug, Clone)]
pub(super) struct CountRelease {
    live: Arc<AtomicUsize>,
    released: Arc<AtomicBool>,
}

impl CountRelease {
    pub(super) fn new(live: Arc<AtomicUsize>) -> Self {
        Self {
            live,
            released: Arc::new(AtomicBool::new(false)),
        }
    }

    /// Release this subscription's slot in the count, if it still holds one.
    pub(super) fn release(&self) {
        if !self.released.swap(true, Ordering::SeqCst) {
            self.live.fetch_sub(1, Ordering::Relaxed);
        }
    }
}

/// A live subscription. Dropping it unsubscribes.
#[derive(Debug)]
pub struct Subscription {
    pub(super) events: mpsc::Receiver<Delivery>,
    /// Cancellation arrives here rather than through the event channel.
    ///
    /// A separate channel because the event channel is allowed to be full —
    /// that is its entire design — and a cancellation that could be dropped for
    /// lack of space is a revocation that silently does not happen.
    pub(super) cancel_rx: mpsc::Receiver<()>,
    /// Kept so the cancel channel always has a live sender, which is what makes
    /// `Ready(None)` on `cancel_rx` unreachable and removes the question of
    /// whether a dropped handle means "cancelled" or "never mind".
    pub(super) cancel_tx: mpsc::Sender<()>,
    pub(super) release: CountRelease,
}

impl Drop for Subscription {
    fn drop(&mut self) {
        self.release.release();
    }
}

impl Subscription {
    /// A handle that can end this subscription from outside it.
    ///
    /// Cloneable and `Send`, because the thing that discovers a revocation is a
    /// separate task from the one serving the stream.
    #[must_use]
    pub fn canceller(&self) -> Canceller {
        Canceller {
            tx: self.cancel_tx.clone(),
            release: self.release.clone(),
        }
    }

    /// The next thing to happen on this subscription.
    pub async fn recv(&mut self) -> Received {
        std::future::poll_fn(|cx| self.poll_recv(cx)).await
    }

    /// The same, for a caller driving this from a `Stream`.
    ///
    /// Exposed as a poll rather than handing out the receiver, so the
    /// subscription and its count slot cannot be separated — a caller holding a
    /// bare receiver would keep receiving events while
    /// `sse_connections_active` had already forgotten it.
    ///
    /// `std::task` rather than a futures trait: this crate holds adapters, and a
    /// `Stream` implementation here would put the choice of async ecosystem in a
    /// crate whose job is to be indifferent to it.
    pub fn poll_recv(&mut self, cx: &mut std::task::Context<'_>) -> std::task::Poll<Received> {
        use std::task::Poll;

        // Cancellation first, and unconditionally. Checking events first would
        // let a subscriber with a full queue drain 64 more events after being
        // revoked — which is exactly the window docs/40 says must not exist.
        match self.cancel_rx.poll_recv(cx) {
            Poll::Ready(_) => {
                // `Some` is a cancel; `None` cannot happen while `cancel_tx`
                // lives here, and if it somehow did, ending the stream is the
                // safe direction.
                self.release.release();
                return Poll::Ready(Received::Cancelled);
            }
            Poll::Pending => {}
        }

        self.events.poll_recv(cx).map(|delivery| match delivery {
            Some(Delivery::Event(event)) => Received::Event(event),
            Some(Delivery::Lagged) => Received::Lagged,
            None => Received::Closed,
        })
    }
}

/// Ends a subscription from outside it.
#[derive(Debug, Clone)]
pub struct Canceller {
    tx: mpsc::Sender<()>,
    release: CountRelease,
}

impl Canceller {
    /// End the subscription. Idempotent, and never blocks.
    ///
    /// The count is released here rather than waiting for the stream task to
    /// notice: a revoked stream stops being a connection the moment it is
    /// revoked, whatever the scheduler is doing.
    /// Whether the subscription still exists.
    ///
    /// A revalidation tick uses this to stop: a client that disconnected must
    /// not leave a task behind querying the database on its behalf forever.
    #[must_use]
    pub fn is_live(&self) -> bool {
        !self.tx.is_closed()
    }

    pub fn cancel(&self) {
        self.release.release();
        // Capacity one. `Full` means a cancellation is already queued and this
        // one is redundant; `Closed` means the stream is already gone. Both are
        // successes for a function whose contract is "this subscription is
        // over".
        let _ = self.tx.try_send(());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::broadcast::{Broadcast, LiveEvent, LocalBroadcast, SUBSCRIBER_QUEUE, Topic};
    use casual_task_model::WorkspaceId;
    use uuid::Uuid;

    fn hub_and_topic() -> (LocalBroadcast, Topic) {
        (
            LocalBroadcast::new(),
            Topic::project(WorkspaceId::new(), Uuid::now_v7()),
        )
    }

    fn an_event() -> LiveEvent {
        LiveEvent {
            id: Uuid::now_v7(),
            aggregate_id: Uuid::now_v7(),
            event_type: "task.updated".to_owned(),
            data: "{}".to_owned(),
        }
    }

    #[tokio::test]
    async fn cancelling_ends_the_stream() {
        // docs/40: "an SSE stream held by that session closes". Without this the
        // stream runs until the client leaves, which makes revocation a
        // statement about future requests only.
        let (hub, topic) = hub_and_topic();
        let mut sub = hub.subscribe(topic);
        let canceller = sub.canceller();

        canceller.cancel();
        assert_eq!(sub.recv().await, Received::Cancelled);
    }

    #[tokio::test]
    async fn cancellation_is_seen_even_with_a_full_event_queue() {
        // The reason cancellation has its own channel. A revoked subscriber
        // whose queue is full must not get to drain 64 more events first — that
        // backlog is exactly the window docs/40 says must not exist.
        let (hub, topic) = hub_and_topic();
        let mut sub = hub.subscribe(topic);
        let canceller = sub.canceller();
        for _ in 0..SUBSCRIBER_QUEUE {
            assert_eq!(hub.publish(topic, an_event()), 1);
        }

        canceller.cancel();
        assert_eq!(
            sub.recv().await,
            Received::Cancelled,
            "a queued event was delivered ahead of a cancellation"
        );
    }

    #[tokio::test]
    async fn a_cancelled_stream_stops_being_counted_before_it_is_dropped() {
        // `sse_connections_active` must count streams that can still receive. A
        // cancelled one cannot, but its task is not dropped until the runtime
        // next polls it — so counting it in between makes the gauge read high
        // during exactly the incident an operator is watching it for.
        let (hub, topic) = hub_and_topic();
        let sub = hub.subscribe(topic);
        assert_eq!(hub.subscriber_count(), 1);

        sub.canceller().cancel();
        assert_eq!(
            hub.subscriber_count(),
            0,
            "a cancelled subscription is still counted as an open connection"
        );

        // And dropping it afterwards must not double-count: releasing twice
        // would take the gauge below the number of live streams, which is worse
        // than the drift it was fixing.
        drop(sub);
        assert_eq!(hub.subscriber_count(), 0);
    }

    #[tokio::test]
    async fn cancelling_twice_is_not_two_releases() {
        let (hub, topic) = hub_and_topic();
        let held = hub.subscribe(topic);
        let sub = hub.subscribe(topic);
        assert_eq!(hub.subscriber_count(), 2);

        let canceller = sub.canceller();
        canceller.cancel();
        canceller.cancel();
        canceller.cancel();
        assert_eq!(
            hub.subscriber_count(),
            1,
            "repeated cancellation released the count more than once"
        );
        drop(held);
        assert_eq!(hub.subscriber_count(), 0);
    }

    #[tokio::test]
    async fn a_live_subscription_is_not_cancelled_by_dropping_a_handle() {
        // The trap in building this on a oneshot: a dropped sender and a sent
        // cancellation look the same to the receiver, so a caller that took a
        // handle and let it fall out of scope would silently kill the stream.
        let (hub, topic) = hub_and_topic();
        let mut sub = hub.subscribe(topic);
        drop(sub.canceller());

        assert_eq!(hub.publish(topic, an_event()), 1);
        assert!(
            matches!(sub.recv().await, Received::Event(_)),
            "dropping a canceller ended a subscription that was never cancelled"
        );
    }
}
