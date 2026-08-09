//! The 100 ms window (`docs/05` §Live updates).
//!
//! # The failure this file exists to prevent
//!
//! One frame per row. A bulk edit across two hundred tasks, or a drag that
//! crosses forty positions, produces an event per change — and every one of them
//! is a frame written to every subscriber on the board, parsed by every browser,
//! and turned into a re-render. The work is quadratic in the wrong thing: rows
//! changed × people watching.
//!
//! > Events are **coalesced** per aggregate over a 100 ms window, so a rapid drag
//! > produces one update, not forty.
//!
//! # Per (aggregate, event type), not per aggregate — and why that is narrower
//!
//! `docs/05` says "per aggregate". This collapses per *aggregate and event
//! type*, which coalesces strictly less.
//!
//! The reason is that collapsing across types loses transitions. `task.created`
//! followed 20 ms later by `task.updated` is two different facts, and keeping
//! only the newer one hands a subscriber an update for a task it has never heard
//! of — the exact shape `docs/25`'s per-aggregate ordering guarantee exists to
//! prevent, reintroduced at the last hop. A drag emits one event type
//! repeatedly, so the case the document is actually about is fully covered.
//!
//! Recorded here rather than resolved silently: if the intent was to collapse
//! across types too, this is the line to change.
//!
//! # Every bound names its overflow policy (`docs/24` §D-040)
//!
//! | Bound | Value | When it is reached |
//! | --- | --- | --- |
//! | Distinct keys buffered | [`MAX_PENDING`] | The window is cut short and everything buffered is emitted immediately. Waiting would grow the buffer; dropping would lose events. Flushing early costs a frame and loses nothing. |
//! | Time held | [`WINDOW`] | Emitted. That is the feature. |
//!
//! The delay is the cost, and it is stated: an event is held for up to
//! [`WINDOW`] before it reaches a client. `docs/30` budgets live updates in
//! seconds, so 100 ms is inside the noise — but it is not zero, and a future
//! requirement for sub-100 ms delivery would have to change this rather than
//! discover it.

use std::time::{Duration, Instant};

use casual_task_infra::broadcast::LiveEvent;
use uuid::Uuid;

/// `docs/05`: "coalesced per aggregate over a 100 ms window".
pub const WINDOW: Duration = Duration::from_millis(100);

/// How many distinct events may be held at once before the window is cut short.
///
/// A bulk operation is capped at 100 tasks (`docs/21`), and a window can span
/// more than one, so 256 leaves room for the documented worst case without
/// letting an unbounded buffer form behind a client that is being helpful by
/// changing a lot at once.
pub const MAX_PENDING: usize = 256;

/// What decides that two events are the same update.
type Key = (Uuid, String);

/// Holds events for [`WINDOW`], keeping the newest per [`Key`].
#[derive(Debug, Default)]
pub struct Coalescer {
    /// Insertion-ordered. A replacement keeps its **original** position, so
    /// distinct aggregates still reach the client in the order their first
    /// change did — collapsing must not reorder what it does not merge.
    pending: Vec<(Key, LiveEvent)>,
    /// When the oldest held event was buffered. `None` when nothing is held.
    opened: Option<Instant>,
}

impl Coalescer {
    /// An empty window.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Buffer `event`. Returns `true` when the caller should flush immediately.
    pub fn push(&mut self, event: LiveEvent, now: Instant) -> bool {
        let key = (event.aggregate_id, event.event_type.clone());
        if let Some(slot) = self.pending.iter_mut().find(|(k, _)| *k == key) {
            // Same task, same kind of change: the newer payload is the whole of
            // the truth, and the older one describes a state no client will ever
            // need to render.
            slot.1 = event;
        } else {
            self.pending.push((key, event));
        }
        if self.opened.is_none() {
            self.opened = Some(now);
        }
        self.pending.len() >= MAX_PENDING
    }

    /// When the held events are due, if any are held.
    #[must_use]
    pub fn due_at(&self) -> Option<Instant> {
        self.opened.map(|opened| opened + WINDOW)
    }

    /// Whether anything is waiting.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.pending.is_empty()
    }

    /// Take everything held, oldest first.
    pub fn drain(&mut self) -> Vec<LiveEvent> {
        self.opened = None;
        self.pending.drain(..).map(|(_, event)| event).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn event_for(aggregate: Uuid, event_type: &str, data: &str) -> LiveEvent {
        LiveEvent {
            id: Uuid::now_v7(),
            aggregate_id: aggregate,
            event_type: event_type.to_owned(),
            data: data.to_owned(),
        }
    }

    #[test]
    fn a_rapid_drag_collapses_to_one_update() {
        // docs/05's own example: "a rapid drag produces one update, not forty".
        let mut window = Coalescer::new();
        let task = Uuid::now_v7();
        let now = Instant::now();

        for position in 0..40 {
            let flush = window.push(
                event_for(task, "task.updated", &format!("{{\"rank\":{position}}}")),
                now,
            );
            assert!(
                !flush,
                "forty events on one task should not fill the buffer"
            );
        }

        let emitted = window.drain();
        assert_eq!(emitted.len(), 1, "forty drag frames were not collapsed");
        assert_eq!(
            emitted[0].data, "{\"rank\":39}",
            "coalescing kept an intermediate position instead of the final one"
        );
    }

    #[test]
    fn different_tasks_are_not_collapsed_into_each_other() {
        // The counterweight: a coalescer that merged everything would pass the
        // test above and lose every event but one during a bulk edit.
        let mut window = Coalescer::new();
        let now = Instant::now();
        for _ in 0..10 {
            window.push(event_for(Uuid::now_v7(), "task.updated", "{}"), now);
        }
        assert_eq!(window.drain().len(), 10);
    }

    #[test]
    fn a_create_is_not_swallowed_by_a_later_update() {
        // Collapsing across event types would hand a subscriber an update for a
        // task it has never heard of — docs/25's ordering guarantee undone at
        // the last hop.
        let mut window = Coalescer::new();
        let task = Uuid::now_v7();
        let now = Instant::now();
        window.push(event_for(task, "task.created", "{}"), now);
        window.push(event_for(task, "task.updated", "{}"), now);

        let emitted = window.drain();
        assert_eq!(
            emitted
                .iter()
                .map(|e| e.event_type.as_str())
                .collect::<Vec<_>>(),
            vec!["task.created", "task.updated"],
            "a create was collapsed into the update that followed it"
        );
    }

    #[test]
    fn collapsing_does_not_reorder_what_it_does_not_merge() {
        // A replacement that moved its entry to the end would deliver an older
        // task's first change after a newer task's, which is a reordering the
        // client cannot detect and cannot correct.
        let mut window = Coalescer::new();
        let first = Uuid::now_v7();
        let second = Uuid::now_v7();
        let now = Instant::now();

        window.push(event_for(first, "task.updated", "a"), now);
        window.push(event_for(second, "task.updated", "b"), now);
        window.push(event_for(first, "task.updated", "c"), now);

        let emitted = window.drain();
        assert_eq!(
            emitted.iter().map(|e| e.data.as_str()).collect::<Vec<_>>(),
            vec!["c", "b"],
            "the replaced entry jumped position"
        );
    }

    #[test]
    fn the_window_opens_on_the_first_event_and_not_the_last() {
        // A deadline that reset on every push would never fire under a sustained
        // edit: the client would receive nothing until the storm stopped.
        let mut window = Coalescer::new();
        let start = Instant::now();
        window.push(event_for(Uuid::now_v7(), "task.updated", "{}"), start);
        let due = window.due_at().expect("a held event is due at some point");

        window.push(
            event_for(Uuid::now_v7(), "task.updated", "{}"),
            start + Duration::from_millis(50),
        );
        assert_eq!(
            window.due_at(),
            Some(due),
            "a later event pushed the deadline out; a busy board would starve"
        );
        assert_eq!(due, start + WINDOW);
    }

    #[test]
    fn the_buffer_is_bounded_and_says_so_by_asking_for_a_flush() {
        // D-040: the bound has a policy, and the policy is "emit now". Growing
        // would be unbounded memory; dropping would lose events.
        let mut window = Coalescer::new();
        let now = Instant::now();
        let mut asked = false;
        for _ in 0..MAX_PENDING {
            asked = window.push(event_for(Uuid::now_v7(), "task.updated", "{}"), now);
        }
        assert!(
            asked,
            "the buffer reached {MAX_PENDING} entries without asking to flush"
        );
        assert_eq!(window.drain().len(), MAX_PENDING, "a flush lost events");
    }

    #[test]
    fn draining_closes_the_window() {
        let mut window = Coalescer::new();
        window.push(
            event_for(Uuid::now_v7(), "task.updated", "{}"),
            Instant::now(),
        );
        assert!(window.due_at().is_some());
        let _ = window.drain();
        assert!(window.is_empty());
        assert_eq!(
            window.due_at(),
            None,
            "an empty window still had a deadline, so the stream would wake for nothing"
        );
    }
}
