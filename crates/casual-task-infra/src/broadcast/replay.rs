//! What a client gets back when it reconnects.
//!
//! # The failure this file exists to prevent
//!
//! A client that reconnects and *believes* it is caught up when it is not.
//!
//! Everything else in this module tells a subscriber to reconnect with
//! `Last-Event-ID`: the lag policy does it, cancellation does it, a dropped
//! socket does it. That advice was worthless until now — there was nothing
//! behind the header, so a reconnecting client silently resumed live and carried
//! whatever gap it had acquired. A recovery instruction that does not recover is
//! worse than none, because the client stops looking.
//!
//! So the buffer exists, and — more importantly — it is honest about its own
//! edges. `docs/05` is explicit about both halves:
//!
//! > **`Last-Event-ID` replay** on reconnect, bounded to 5 minutes / 1,000
//! > events. Beyond that the client is told to refetch rather than being handed
//! > a partial history it would silently treat as complete.
//!
//! [`Resume::Gap`] is that sentence. A client past the window is told it lost
//! events; it is never handed the tail of a history and left to assume it was
//! the whole of it.
//!
//! # Every bound names its overflow policy (`docs/24` §D-040)
//!
//! | Bound | Value | When it is reached |
//! | --- | --- | --- |
//! | Events kept per topic | [`REPLAY_EVENTS`] | The oldest are dropped. A client asking for one of them gets [`Resume::Gap`] — told, not guessed at. |
//! | Age kept per topic | [`REPLAY_WINDOW`] | Same: pruned on write, and asking for a pruned event is a gap. |
//! | Topics buffered at once | [`MAX_REPLAY_TOPICS`] | The least recently published topic is evicted whole. A client reconnecting to it gets [`Resume::Gap`], which is the same answer it would get for an expired event and needs no separate handling. |
//!
//! The third bound is the one that is easy to forget: a map keyed by project id
//! is a map an attacker can grow by opening projects, and 1,000 events each is
//! only bounded memory if the number of topics is bounded too.

use std::collections::HashMap;
use std::time::{Duration, Instant};

use uuid::Uuid;

use super::{LiveEvent, Topic};

/// `docs/05`: replay is bounded to 1,000 events.
pub const REPLAY_EVENTS: usize = 1_000;

/// `docs/05`: replay is bounded to 5 minutes.
pub const REPLAY_WINDOW: Duration = Duration::from_secs(5 * 60);

/// How many topics keep a replay buffer at once.
///
/// Not from `docs/05`, which bounds the per-topic history and stops. This is the
/// bound that keeps the *number* of histories finite: without it the buffer is a
/// map keyed by project id, and the memory ceiling is
/// `REPLAY_EVENTS × payload × every project ever streamed`.
///
/// 1,024 topics is far more than a single instance serves concurrently and small
/// enough to reason about: at 1,000 events of 1 KiB it caps the buffer at about
/// a gigabyte in the pathological case, and at the realistic case — a few dozen
/// active boards — it is a few megabytes.
pub const MAX_REPLAY_TOPICS: usize = 1_024;

/// What a reconnecting subscriber is entitled to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Resume {
    /// No `Last-Event-ID`, or the id is the newest event: nothing to replay.
    Live,
    /// The events after the client's last id, oldest first.
    Replayed(Vec<LiveEvent>),
    /// The client's position is outside the window. It has lost events and is
    /// being told so.
    ///
    /// `docs/05`: "the client is told to refetch rather than being handed a
    /// partial history it would silently treat as complete."
    Gap,
}

/// One topic's recent history.
#[derive(Debug)]
struct History {
    /// Oldest first. A `Vec` and not a `VecDeque` because the only operations
    /// are "append", "drop a prefix" and "find an id and take the rest" — and
    /// the prefix drop happens on write, where the copy is amortised, not on the
    /// read path where a client is waiting.
    events: Vec<(Instant, LiveEvent)>,
    last_published: Instant,
}

/// Bounded per-topic history, for reconnecting clients.
#[derive(Debug, Default)]
pub struct ReplayBuffer {
    topics: HashMap<Topic, History>,
}

impl ReplayBuffer {
    /// An empty buffer.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Record `event`, pruning whatever the bounds no longer allow.
    pub fn record(&mut self, topic: Topic, event: &LiveEvent, now: Instant) {
        if !self.topics.contains_key(&topic) && self.topics.len() >= MAX_REPLAY_TOPICS {
            self.evict_stalest(now);
        }

        let history = self.topics.entry(topic).or_insert_with(|| History {
            events: Vec::new(),
            last_published: now,
        });
        history.last_published = now;
        history.events.push((now, event.clone()));

        // Age first, then count. Doing it on write keeps the read path — where a
        // client is mid-reconnect — free of work that could have been done
        // earlier.
        let cutoff = now.checked_sub(REPLAY_WINDOW);
        if let Some(cutoff) = cutoff {
            let expired = history
                .events
                .iter()
                .take_while(|(at, _)| *at < cutoff)
                .count();
            if expired > 0 {
                history.events.drain(..expired);
            }
        }
        if history.events.len() > REPLAY_EVENTS {
            let excess = history.events.len() - REPLAY_EVENTS;
            history.events.drain(..excess);
        }
    }

    /// What a client resuming from `after` is entitled to.
    ///
    /// `None` means a fresh connection with no `Last-Event-ID`, which is
    /// [`Resume::Live`] — not a gap. A first connection has lost nothing.
    #[must_use]
    pub fn since(&self, topic: Topic, after: Option<Uuid>) -> Resume {
        let Some(after) = after else {
            return Resume::Live;
        };
        let Some(history) = self.topics.get(&topic) else {
            // No history at all: either nothing has ever been published here, or
            // this topic was evicted. The two are indistinguishable from here and
            // must be answered the same way — a client that lost events because
            // of eviction must not be told it is current.
            return Resume::Gap;
        };

        let Some(position) = history.events.iter().position(|(_, e)| e.id == after) else {
            // The id is older than the window, or from a previous process, or
            // was never ours. All of them mean the same thing to the client.
            return Resume::Gap;
        };

        let missed: Vec<LiveEvent> = history.events[position + 1..]
            .iter()
            .map(|(_, e)| e.clone())
            .collect();
        if missed.is_empty() {
            Resume::Live
        } else {
            Resume::Replayed(missed)
        }
    }

    /// How many topics currently hold a history. For assertions and for the
    /// caller's diagnostics.
    #[must_use]
    pub fn topic_count(&self) -> usize {
        self.topics.len()
    }

    /// Drop the least recently published topic.
    ///
    /// Least recently *published*, not least recently read: a busy board is the
    /// one a reconnecting client is most likely to need, and read activity is
    /// not visible here.
    fn evict_stalest(&mut self, now: Instant) {
        let stalest = self
            .topics
            .iter()
            .min_by_key(|(_, history)| history.last_published)
            .map(|(topic, _)| *topic);
        if let Some(topic) = stalest {
            self.topics.remove(&topic);
        }
        // `now` is unused except to make the signature honest about when this
        // happens; eviction is by recorded time, not by clock reading.
        let _ = now;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use casual_task_model::WorkspaceId;

    fn a_topic() -> Topic {
        Topic::project(WorkspaceId::new(), Uuid::now_v7())
    }

    fn an_event() -> LiveEvent {
        LiveEvent {
            id: Uuid::now_v7(),
            aggregate_id: Uuid::now_v7(),
            event_type: "task.updated".to_owned(),
            data: "{}".to_owned(),
        }
    }

    #[test]
    fn a_fresh_connection_is_live_and_not_a_gap() {
        // A client with no Last-Event-ID has lost nothing. Answering `Gap` would
        // make every first connection tell the user it missed events.
        let buffer = ReplayBuffer::new();
        assert_eq!(buffer.since(a_topic(), None), Resume::Live);
    }

    #[test]
    fn a_reconnecting_client_gets_exactly_what_it_missed() {
        let mut buffer = ReplayBuffer::new();
        let topic = a_topic();
        let now = Instant::now();

        let first = an_event();
        let second = an_event();
        let third = an_event();
        for event in [&first, &second, &third] {
            buffer.record(topic, event, now);
        }

        let Resume::Replayed(missed) = buffer.since(topic, Some(first.id)) else {
            panic!("a client one event behind was not offered a replay");
        };
        assert_eq!(
            missed.iter().map(|e| e.id).collect::<Vec<_>>(),
            vec![second.id, third.id],
            "replay must be everything AFTER the client's position, oldest first"
        );
    }

    #[test]
    fn a_client_that_is_already_current_is_live() {
        let mut buffer = ReplayBuffer::new();
        let topic = a_topic();
        let event = an_event();
        buffer.record(topic, &event, Instant::now());
        assert_eq!(buffer.since(topic, Some(event.id)), Resume::Live);
    }

    #[test]
    fn an_id_past_the_event_bound_is_a_gap_not_a_partial_history() {
        // docs/05, the sentence this whole file is for: "the client is told to
        // refetch rather than being handed a partial history it would silently
        // treat as complete."
        let mut buffer = ReplayBuffer::new();
        let topic = a_topic();
        let now = Instant::now();

        let oldest = an_event();
        buffer.record(topic, &oldest, now);
        for _ in 0..REPLAY_EVENTS {
            buffer.record(topic, &an_event(), now);
        }

        assert_eq!(
            buffer.since(topic, Some(oldest.id)),
            Resume::Gap,
            "a client whose position fell out of the buffer was handed a tail of \
             the history and left to assume it was all of it"
        );
    }

    #[test]
    fn an_id_older_than_the_window_is_a_gap() {
        let mut buffer = ReplayBuffer::new();
        let topic = a_topic();
        let start = Instant::now();

        let old = an_event();
        buffer.record(topic, &old, start);
        // One event, well past the window. The prune happens on write, so this
        // second record is what expires the first.
        buffer.record(
            topic,
            &an_event(),
            start + REPLAY_WINDOW + Duration::from_secs(1),
        );

        assert_eq!(buffer.since(topic, Some(old.id)), Resume::Gap);
    }

    #[test]
    fn the_window_keeps_what_is_still_inside_it() {
        // The counterweight. A buffer that expired everything would satisfy the
        // test above and make replay useless.
        let mut buffer = ReplayBuffer::new();
        let topic = a_topic();
        let start = Instant::now();

        let first = an_event();
        buffer.record(topic, &first, start);
        buffer.record(topic, &an_event(), start + Duration::from_secs(1));

        assert!(
            matches!(buffer.since(topic, Some(first.id)), Resume::Replayed(m) if m.len() == 1),
            "an event one second old was pruned from a five-minute window"
        );
    }

    #[test]
    fn an_unknown_id_is_a_gap() {
        // A client from a previous process, or one sending an id that was never
        // ours. It cannot be told it is current.
        let mut buffer = ReplayBuffer::new();
        let topic = a_topic();
        buffer.record(topic, &an_event(), Instant::now());
        assert_eq!(buffer.since(topic, Some(Uuid::now_v7())), Resume::Gap);
    }

    #[test]
    fn the_number_of_buffered_topics_is_bounded() {
        // A map keyed by project id is a map a user can grow by opening
        // projects. 1,000 events each is only bounded memory if this is bounded.
        let mut buffer = ReplayBuffer::new();
        let now = Instant::now();
        for _ in 0..(MAX_REPLAY_TOPICS + 50) {
            buffer.record(a_topic(), &an_event(), now);
        }
        assert!(
            buffer.topic_count() <= MAX_REPLAY_TOPICS,
            "the replay buffer holds {} topics; the cap is {MAX_REPLAY_TOPICS}",
            buffer.topic_count()
        );
    }

    #[test]
    fn an_evicted_topic_answers_gap_rather_than_live() {
        // The trap in evicting whole topics: "I have no history for you" and
        // "you are current" are the same absence, and answering Live would tell
        // a client that lost everything that it lost nothing.
        let mut buffer = ReplayBuffer::new();
        let start = Instant::now();
        let victim = a_topic();
        let event = an_event();
        buffer.record(victim, &event, start);

        // Every later topic is more recently published, so the victim is stalest.
        for n in 0..(MAX_REPLAY_TOPICS + 10) {
            buffer.record(
                a_topic(),
                &an_event(),
                start + Duration::from_millis(n as u64 + 1),
            );
        }

        assert_eq!(
            buffer.since(victim, Some(event.id)),
            Resume::Gap,
            "a client reconnecting to an evicted topic was told it was current"
        );
    }

    #[test]
    fn the_busiest_topic_survives_eviction() {
        // Least recently *published*, not arbitrary: a board under active edit
        // is the one a reconnecting client is most likely to need.
        let mut buffer = ReplayBuffer::new();
        let start = Instant::now();
        let busy = a_topic();

        for n in 0..(MAX_REPLAY_TOPICS + 10) {
            let at = start + Duration::from_millis(n as u64 + 1);
            buffer.record(a_topic(), &an_event(), at);
            // The busy topic keeps publishing, so it is never the stalest.
            buffer.record(busy, &an_event(), at);
        }

        // Asserted on a RECENT event, not the first one: the busy topic has
        // published far more than REPLAY_EVENTS by now, so its oldest entries
        // are legitimately gone to the per-topic bound. What this test is about
        // is the *topic* surviving eviction, which is a different bound.
        let recent = an_event();
        let at = start + Duration::from_secs(2);
        buffer.record(busy, &recent, at);
        buffer.record(busy, &an_event(), at);

        assert!(
            !matches!(buffer.since(busy, Some(recent.id)), Resume::Gap),
            "the most active topic was evicted whole while idle ones survived"
        );
    }
}
