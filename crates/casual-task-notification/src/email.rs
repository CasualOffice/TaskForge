//! What a notification email says (`docs/29` §Email content).
//!
//! # The failure this module prevents
//!
//! Mail that says "something changed". `docs/29`: an email "renders the change,
//! not just 'something changed'" — a notification the recipient cannot act on
//! without opening the product is a notification that trains them to ignore the
//! next one.
//!
//! The second failure is quieter: a subject that does not thread. `docs/29`
//! fixes it as `[WR-125] Task title` and says why — "stable, so mail clients
//! thread correctly". Stable means the *same task* always produces the *same*
//! subject, which is why [`subject`] takes the key and title and nothing that
//! varies per event.
//!
//! # Composition, not transport
//!
//! This module returns text. It holds no `Mailer`, opens no connection and
//! knows no SMTP — `docs/19` gives this crate "recipient and reason
//! computation" and says channel transport "belongs to the worker and
//! `casual-task-infra`". So the worker sends what this composes, and every rule
//! about what a notification *says* is testable without a relay.

use crate::reason::Reason;

/// The task an email is about, in the only terms this module needs.
///
/// Deliberately not a task row: composition must not depend on the task
/// aggregate's shape, and the fields below are the ones `docs/29` names.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Subject<'a> {
    /// The human key, `WR-125`.
    pub key: &'a str,
    /// The task title. **Customer content** — see the module docs of
    /// `casual-task-infra::mail` for where that constrains it.
    pub title: &'a str,
}

/// Who did the thing, for the sentence that renders the change.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Actor<'a> {
    pub display_name: &'a str,
}

/// `[WR-125] Task title` — `docs/29` §Email content.
///
/// Stable across every event about the task, which is what makes a mail client
/// thread them. A subject that carried the event type would put each change in
/// its own thread.
#[must_use]
pub fn subject(about: &Subject<'_>) -> String {
    format!("[{}] {}", about.key, about.title)
}

/// The plain-text body.
///
/// Three parts, in the order a reader needs them: what happened, why they are
/// being told, and where to go. `docs/29` requires the first and the third; the
/// second is what makes an unsubscribe decision possible without opening the
/// product.
#[must_use]
pub fn body(
    about: &Subject<'_>,
    actor: Option<Actor<'_>>,
    reason: Reason,
    event_type: &str,
    task_url: &str,
) -> String {
    let who = actor.map_or_else(
        // A system-generated event has no actor (migration 0024). "TaskForge"
        // rather than "somebody": the recipient should not be left wondering
        // which colleague it was.
        || "TaskForge".to_owned(),
        |actor| actor.display_name.to_owned(),
    );
    format!(
        "{who} {change} {key}: {title}\n\n\
         You are receiving this because {why}.\n\n\
         {url}\n",
        change = rendered_change(event_type),
        key = about.key,
        title = about.title,
        why = explanation(reason),
        url = task_url,
    )
}

/// The change, in words. `docs/29`: "Renders the change, not just 'something
/// changed.'"
///
/// An unknown event type falls back to a generic phrase rather than being
/// refused: the fan-out only subscribes to the events it understands, so
/// reaching this means a new event type shipped ahead of its wording, and a
/// vaguer email is better than a lost one.
fn rendered_change(event_type: &str) -> &'static str {
    match event_type {
        "task.created" => "created",
        "task.updated" => "updated",
        "task.assigned" => "assigned you to",
        "task.unassigned" => "unassigned you from",
        "task.status.changed" => "changed the status of",
        "task.closed" => "closed",
        "task.reopened" => "reopened",
        "task.deleted" => "deleted",
        "task.tagged" => "tagged",
        "comment.created" => "commented on",
        "comment.updated" => "edited a comment on",
        _ => "changed",
    }
}

/// Why this person is being told, in the second person.
const fn explanation(reason: Reason) -> &'static str {
    match reason {
        Reason::Mentioned => "you were mentioned",
        Reason::Assigned => "you are assigned to this task",
        Reason::Reported => "you reported this task",
        Reason::Subscribed => "you are following this task",
        Reason::Participated => "you commented on this task",
        Reason::Team => "your team is watching this project",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn about() -> Subject<'static> {
        Subject {
            key: "WR-125",
            title: "Ship the thing",
        }
    }

    #[test]
    fn the_subject_is_the_documented_shape() {
        // docs/29 §Email content: `[WR-125] Task title`.
        assert_eq!(subject(&about()), "[WR-125] Ship the thing");
    }

    #[test]
    fn the_subject_is_stable_across_every_event_about_one_task() {
        // The threading property. A subject that varied per event would put
        // each change in its own thread, which is the thing docs/29 calls out.
        let first = subject(&about());
        let second = subject(&about());
        assert_eq!(first, second);
    }

    #[test]
    fn the_body_renders_the_change_rather_than_saying_something_changed() {
        // docs/29 is explicit about this, and it is the difference between mail
        // people read and mail people filter.
        let rendered = body(
            &about(),
            Some(Actor {
                display_name: "Sarah Johnson",
            }),
            Reason::Assigned,
            "task.assigned",
            "https://taskforge.example/t/WR-125",
        );
        assert!(rendered.starts_with("Sarah Johnson assigned you to WR-125: Ship the thing"));
        assert!(rendered.contains("you are assigned to this task"));
        assert!(rendered.contains("https://taskforge.example/t/WR-125"));
        assert!(
            !rendered.contains("something changed"),
            "the fallback wording escaped into a known event"
        );
    }

    #[test]
    fn every_reason_has_its_own_explanation() {
        // "You are receiving this because …" is what makes an unsubscribe
        // decision possible from the mail. Two reasons sharing a sentence would
        // make it useless for exactly the reasons people want to mute.
        let mut seen: Vec<&str> = Reason::ALL.iter().map(|r| explanation(*r)).collect();
        let before = seen.len();
        seen.sort_unstable();
        seen.dedup();
        assert_eq!(seen.len(), before, "two reasons share an explanation");
        assert!(seen.iter().all(|s| !s.is_empty()));
    }

    #[test]
    fn every_event_the_fanout_subscribes_to_has_real_wording() {
        // The fallback exists so a new event type cannot lose a notification.
        // It must not be reached by the ones that exist today.
        for event in [
            "task.created",
            "task.updated",
            "task.assigned",
            "task.unassigned",
            "task.status.changed",
            "task.closed",
            "task.reopened",
            "task.deleted",
            "task.tagged",
            "comment.created",
            "comment.updated",
        ] {
            assert_ne!(
                rendered_change(event),
                "changed",
                "{event} has no wording of its own"
            );
        }
        assert_eq!(rendered_change("plugin.invented.tomorrow"), "changed");
    }

    #[test]
    fn a_system_event_is_attributed_to_the_product_not_to_nobody() {
        // `actor_id` is NULL for a system-generated event. "assigned you to" with
        // an empty name in front of it reads as a bug; leaving the recipient
        // guessing which colleague did it is worse.
        let rendered = body(
            &about(),
            None,
            Reason::Reported,
            "task.updated",
            "https://example.test/t",
        );
        assert!(
            rendered.starts_with("TaskForge updated WR-125"),
            "{rendered}"
        );
    }

    #[test]
    fn the_title_appears_exactly_where_the_contract_says_and_nowhere_else() {
        // The title is customer content. It belongs in the subject and in the
        // first line of the body (docs/29), and it must not leak into the
        // explanation or the URL, which are the parts a future change might
        // start logging.
        let rendered = body(
            &about(),
            Some(Actor {
                display_name: "Sarah",
            }),
            Reason::Mentioned,
            "comment.created",
            "https://example.test/t",
        );
        let lines: Vec<&str> = rendered.lines().filter(|l| !l.is_empty()).collect();
        assert!(lines[0].contains("Ship the thing"));
        assert!(!lines[1].contains("Ship the thing"), "{:?}", lines[1]);
        assert!(!lines[2].contains("Ship the thing"), "{:?}", lines[2]);
    }
}
