//! The in-process fan-out (`docs/48` Profile 1).
//!
//! # The failure this file exists to prevent
//!
//! A client that stops reading must cost the server a bounded amount and then
//! be disconnected. Everything here is arranged around that: a per-subscriber
//! queue with a fixed capacity, a `try_send` that never waits, and an overflow
//! path that ends the subscription rather than growing it.
//!
//! The alternative — one shared queue, or an unbounded per-client one — fails in
//! the way that is hardest to see coming: memory climbs in proportion to the
//! slowest client on the worst network, and the process dies during the traffic
//! spike that produced the slow client in the first place.

use std::collections::HashMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use tokio::sync::mpsc;

use super::subscription::{CountRelease, Delivery};
use super::{Broadcast, LiveEvent, Subscription, Topic};

/// How many events may be queued for one subscriber before it is dropped.
///
/// # Why 64, and what happens at 65
///
/// `docs/24` §D-040 requires the number *and* the policy. The policy is
/// [`Subscription::Lagged`]: the subscription ends, the HTTP stream closes, and
/// the client reconnects with `Last-Event-ID` — which is a recovery path
/// `docs/05` already specifies, so a disconnect costs a round trip and not a
/// correctness hole.
///
/// 64 is sized against the coalescing window in `docs/05`: events are coalesced
/// per aggregate over 100 ms, so a subscriber has to be roughly six seconds
/// behind on a project changing continuously before it overflows. A client that
/// far behind is not slow, it is gone.
pub const SUBSCRIBER_QUEUE: usize = 64;

/// The most subscriptions one process will hold open.
///
/// A second bound behind the per-subscriber one: sixty-four events each is only
/// bounded memory if the number of subscribers is bounded too. At the cap
/// [`LocalBroadcast::subscribe`] still returns a [`Subscription`], but a closed
/// one — the caller sees the stream end immediately, which surfaces as a client
/// retrying against a server that is at capacity rather than as an OOM.
pub const MAX_SUBSCRIBERS: usize = 10_000;

/// Channel slots held back from events so the lag notice always fits.
///
/// One. Without it the overflow path cannot report itself: the notice is sent
/// on the same channel that just proved to be full.
const RESERVED: usize = 1;

/// Fan-out inside one process.
#[derive(Debug, Default)]
pub struct LocalBroadcast {
    topics: Mutex<HashMap<Topic, Vec<mpsc::Sender<Delivery>>>>,
    live: Arc<AtomicUsize>,
}

impl LocalBroadcast {
    /// An empty hub.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// A subscription that is already over, for the at-capacity case.
    ///
    /// Its event channel has no sender, so the first poll reports
    /// [`super::Received::Closed`] and the caller's stream ends immediately.
    fn refused(&self) -> Subscription {
        let (events_tx, events) = mpsc::channel(1);
        drop(events_tx);
        let (cancel_tx, cancel_rx) = mpsc::channel(1);
        // A release that was never counted: `subscribe` decrements before
        // calling this, so releasing again here would take the count below the
        // number of live subscriptions.
        let release = CountRelease::new(Arc::new(AtomicUsize::new(1)));
        Subscription {
            events,
            cancel_rx,
            cancel_tx,
            release,
        }
    }
}

impl Broadcast for LocalBroadcast {
    fn publish(&self, topic: Topic, event: LiveEvent) -> usize {
        let mut topics = self.topics.lock().unwrap_or_else(|p| p.into_inner());
        let Some(subscribers) = topics.get_mut(&topic) else {
            return 0;
        };

        let mut sent = 0;
        // `retain` rather than a loop with a removal list: a subscriber that has
        // gone away or fallen behind is removed in the same pass that finds it,
        // so a dead subscriber cannot be visited twice.
        subscribers.retain(|tx| {
            // The last slot is reserved for the lag notice — see RESERVED. The
            // first version of this let events fill the channel completely and
            // then tried to `try_send(Lagged)` into it, which of course also
            // failed: the subscriber was dropped without ever being told why,
            // and the client saw a stream that ended cleanly. It had no reason
            // to resume from `Last-Event-ID`, so it silently carried on with a
            // hole in its history — the one outcome this policy exists to
            // prevent, produced by the code meant to prevent it.
            if tx.capacity() <= RESERVED {
                // The overflow policy. Blocking instead would let one slow
                // client stall fan-out for every other subscriber on the topic,
                // turning a bad network into a server-wide stall.
                let _ = tx.try_send(Delivery::Lagged);
                return false;
            }
            match tx.try_send(Delivery::Event(Box::new(event.clone()))) {
                Ok(()) => {
                    sent += 1;
                    true
                }
                // Unreachable given the check above, and handled rather than
                // unwrapped: a capacity race must drop the subscriber, not panic
                // the publisher for every other subscriber on the topic.
                Err(mpsc::error::TrySendError::Full(_)) => false,
                // The receiver is gone — the client disconnected.
                Err(mpsc::error::TrySendError::Closed(_)) => false,
            }
        });
        if subscribers.is_empty() {
            // A project nobody watches must not leave an entry behind, or the
            // map grows with every project ever opened until the process ends.
            topics.remove(&topic);
        }
        sent
    }

    fn subscribe(&self, topic: Topic) -> Subscription {
        // Counted before the capacity check, and the check reads the count it
        // just produced: two threads subscribing at the cap must not both
        // conclude there was room.
        let live = self.live.fetch_add(1, Ordering::Relaxed) + 1;
        if live > MAX_SUBSCRIBERS {
            self.live.fetch_sub(1, Ordering::Relaxed);
            return self.refused();
        }

        let (events_tx, events) = mpsc::channel(SUBSCRIBER_QUEUE + RESERVED);
        // Capacity one: a second cancellation of an already-cancelled
        // subscription is redundant, not something to queue.
        let (cancel_tx, cancel_rx) = mpsc::channel(1);
        self.topics
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .entry(topic)
            .or_default()
            .push(events_tx);
        Subscription {
            events,
            cancel_rx,
            cancel_tx,
            release: CountRelease::new(Arc::clone(&self.live)),
        }
    }

    fn subscriber_count(&self) -> usize {
        self.live.load(Ordering::Relaxed)
    }

    fn close_all(&self) {
        // Dropping every sender closes every receiver, which each stream sees as
        // `Received::Closed` and turns into an orderly end-of-stream. D-041: a
        // stream is closed, not dropped mid-frame.
        self.topics
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::broadcast::Received;
    use casual_task_model::WorkspaceId;
    use uuid::Uuid;

    fn an_event(n: u8) -> LiveEvent {
        LiveEvent {
            id: Uuid::now_v7(),
            event_type: "task.updated".to_owned(),
            data: format!("{{\"n\":{n}}}"),
        }
    }

    #[tokio::test]
    async fn a_subscriber_receives_its_own_topic() {
        let hub = LocalBroadcast::new();
        let topic = Topic::project(WorkspaceId::new(), Uuid::now_v7());
        let mut sub = hub.subscribe(topic);

        assert_eq!(hub.publish(topic, an_event(1)), 1);
        assert!(matches!(sub.recv().await, Received::Event(_)));
    }

    #[tokio::test]
    async fn an_event_never_crosses_a_workspace() {
        // The leak with the widest blast radius in the product. Two workspaces
        // holding the same project id must not share a stream — which is not
        // supposed to be possible, and "not supposed to" is not the strength of
        // guarantee docs/32 accepts for tenant isolation.
        let hub = LocalBroadcast::new();
        let project = Uuid::now_v7();
        let alpha = Topic::project(WorkspaceId::new(), project);
        let beta = Topic::project(WorkspaceId::new(), project);

        let mut listener = hub.subscribe(alpha);
        assert_eq!(
            hub.publish(beta, an_event(1)),
            0,
            "an event published in one workspace reached a subscriber in another"
        );

        // And the subscriber really is still live — a test that only asserted
        // "received nothing" would pass against a hub that delivers nothing at
        // all.
        assert_eq!(hub.publish(alpha, an_event(2)), 1);
        assert!(matches!(listener.recv().await, Received::Event(_)));
    }

    #[tokio::test]
    async fn an_event_never_crosses_a_project() {
        let hub = LocalBroadcast::new();
        let workspace = WorkspaceId::new();
        let mine = Topic::project(workspace, Uuid::now_v7());
        let theirs = Topic::project(workspace, Uuid::now_v7());

        let mut listener = hub.subscribe(mine);
        assert_eq!(hub.publish(theirs, an_event(1)), 0);
        assert_eq!(hub.publish(mine, an_event(2)), 1);
        assert!(matches!(listener.recv().await, Received::Event(_)));
    }

    #[tokio::test]
    async fn a_slow_subscriber_is_disconnected_rather_than_queued() {
        // D-040's overflow policy, asserted. Without it this loop is an
        // allocation per event for as long as the client stays connected and
        // silent — a memory-exhaustion primitive any browser can trigger by
        // being slow.
        let hub = LocalBroadcast::new();
        let topic = Topic::project(WorkspaceId::new(), Uuid::now_v7());
        let mut sub = hub.subscribe(topic);

        for n in 0..SUBSCRIBER_QUEUE {
            assert_eq!(
                hub.publish(topic, an_event(n as u8)),
                1,
                "the queue refused an event before reaching its capacity"
            );
        }
        // One past the bound: nothing is queued and the subscriber is dropped.
        assert_eq!(
            hub.publish(topic, an_event(0)),
            0,
            "the subscriber was still accepted past the bound, so the queue grew"
        );
        assert_eq!(
            hub.subscriber_count(),
            1,
            "the count tracks live subscriptions, which this one still is until \
             its receiver is dropped"
        );

        // The receiver drains its backlog and then learns it was dropped, which
        // is what tells the client to resume from Last-Event-ID rather than
        // silently missing events.
        for _ in 0..SUBSCRIBER_QUEUE {
            assert!(matches!(sub.recv().await, Received::Event(_)));
        }
        assert_eq!(
            sub.recv().await,
            Received::Lagged,
            "a dropped subscriber was not told why, so the client cannot know it \
             has a hole in its history"
        );
    }

    #[tokio::test]
    async fn the_subscriber_count_returns_to_zero() {
        // `sse_connections_active` is a gauge, and a gauge that only counts up
        // is one an operator stops trusting after the first deploy.
        let hub = LocalBroadcast::new();
        let topic = Topic::project(WorkspaceId::new(), Uuid::now_v7());
        assert_eq!(hub.subscriber_count(), 0);

        let a = hub.subscribe(topic);
        let b = hub.subscribe(topic);
        assert_eq!(hub.subscriber_count(), 2);

        drop(a);
        assert_eq!(hub.subscriber_count(), 1);
        drop(b);
        assert_eq!(
            hub.subscriber_count(),
            0,
            "a closed stream is still counted; the gauge drifts up forever"
        );
    }

    #[tokio::test]
    async fn closing_the_hub_ends_streams_rather_than_dropping_them() {
        // D-041: SIGTERM closes streams. A client that sees the stream end
        // reconnects; one whose socket vanishes mid-frame sees a parse error.
        let hub = LocalBroadcast::new();
        let topic = Topic::project(WorkspaceId::new(), Uuid::now_v7());
        let mut sub = hub.subscribe(topic);

        hub.close_all();
        assert_eq!(sub.recv().await, Received::Closed);
    }

    #[tokio::test]
    async fn subscriptions_are_capped() {
        // The second bound. Sixty-four events each is bounded memory only if the
        // number of subscribers is bounded too.
        let hub = LocalBroadcast::new();
        let topic = Topic::project(WorkspaceId::new(), Uuid::now_v7());
        let mut held = Vec::new();
        for _ in 0..MAX_SUBSCRIBERS {
            held.push(hub.subscribe(topic));
        }
        assert_eq!(hub.subscriber_count(), MAX_SUBSCRIBERS);

        let mut refused = hub.subscribe(topic);
        assert_eq!(
            refused.recv().await,
            Received::Closed,
            "the hub accepted a subscription past its cap"
        );
        assert_eq!(
            hub.subscriber_count(),
            MAX_SUBSCRIBERS,
            "a refused subscription was counted, so the cap ratchets down"
        );
    }

    #[tokio::test]
    async fn a_topic_with_no_subscribers_leaves_no_entry_behind() {
        // Otherwise the map grows with every project ever opened, which is an
        // unbounded map keyed by something a user chooses — the same shape as
        // the rate limiter's, and the same reason it is not allowed.
        let hub = LocalBroadcast::new();
        let topic = Topic::project(WorkspaceId::new(), Uuid::now_v7());
        drop(hub.subscribe(topic));

        // The publish is what notices the receiver is gone.
        assert_eq!(hub.publish(topic, an_event(1)), 0);
        assert!(
            hub.topics
                .lock()
                .expect("not poisoned")
                .get(&topic)
                .is_none(),
            "an abandoned topic stayed in the map"
        );
    }
}
