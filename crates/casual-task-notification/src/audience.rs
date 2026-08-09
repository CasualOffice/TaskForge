//! Who gets told, out of everyone who could be (`docs/29` §Delivery).
//!
//! # The failure this module prevents
//!
//! Notifying somebody about their own action. `docs/29` calls it out as rule 1
//! and says why it is worth stating: it is "omitted often enough" that it is the
//! single most common complaint about every tracker — you assign a task and your
//! own inbox lights up.
//!
//! It is enforced here, once, in [`resolve`], rather than at each of the six
//! places a candidate can come from. A caller cannot skip it: [`resolve`] is the
//! only constructor of a [`Delivery`], and it takes the actor as a required
//! argument rather than as an option a caller can leave out.
//!
//! # This module performs no I/O and knows no SQL
//!
//! Candidates arrive already loaded and already permission-filtered. That split
//! is what lets every rule in `docs/29` §Batching and suppression be tested
//! without a database — and it is why the *permission* half lives in the
//! persistence query rather than here: a filter applied after the fact is a
//! filter somebody can forget to apply.

use std::collections::BTreeMap;

use casual_task_model::UserId;

use crate::reason::{self, Reason};

/// One person who *might* be notified, and why they might.
///
/// "Might" is the whole point: a candidate is an input to [`resolve`], not a
/// decision. The same user appears once per applicable reason.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Candidate {
    pub user: UserId,
    pub reason: Reason,
}

impl Candidate {
    #[must_use]
    pub const fn new(user: UserId, reason: Reason) -> Self {
        Self { user, reason }
    }
}

/// One person who **will** be notified, and the single reason recorded.
///
/// Constructible only by [`resolve`]. That is deliberate: a `Delivery` built
/// anywhere else would be one that skipped self-suppression and rank
/// resolution, which are the two rules this crate exists to apply.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Delivery {
    user: UserId,
    reason: Reason,
}

impl Delivery {
    #[must_use]
    pub const fn user(&self) -> UserId {
        self.user
    }

    #[must_use]
    pub const fn reason(&self) -> Reason {
        self.reason
    }

    /// Whether this delivery also sends an immediate email today.
    #[must_use]
    pub const fn emails_immediately(&self) -> bool {
        self.reason.emails_immediately()
    }
}

/// Turn candidates into at most one notification per person.
///
/// Three rules, in order, and the order matters:
///
/// 1. **Self-suppression.** The actor is removed first, so a self-mention
///    cannot survive by out-ranking everything else.
/// 2. **One per person.** Duplicates collapse.
/// 3. **Highest reason wins.** [`reason::highest`] decides which survives.
///
/// `actor` is `None` for a system-generated event (migration 0024 makes
/// `outbox_event.actor_id` nullable for exactly that). Nobody's own action
/// caused it, so nobody is suppressed — which is why this takes an `Option`
/// rather than treating "no actor" as "suppress nobody" by accident.
///
/// The result is ordered by user id, so a caller writing rows and a test
/// asserting them see the same order on every run.
#[must_use]
pub fn resolve(actor: Option<UserId>, candidates: &[Candidate]) -> Vec<Delivery> {
    let mut applicable: BTreeMap<UserId, Vec<Reason>> = BTreeMap::new();

    for candidate in candidates {
        // Rule 1, applied before anything else can rescue the row.
        if Some(candidate.user) == actor {
            continue;
        }
        applicable
            .entry(candidate.user)
            .or_default()
            .push(candidate.reason);
    }

    applicable
        .into_iter()
        .filter_map(|(user, reasons)| {
            reason::highest(&reasons).map(|reason| Delivery { user, reason })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn user() -> UserId {
        UserId::new()
    }

    #[test]
    fn nobody_is_notified_about_their_own_action() {
        // docs/29 rule 1, and the reason this crate exists as a separate thing
        // from the queries that find candidates.
        let actor = user();
        let bystander = user();
        let deliveries = resolve(
            Some(actor),
            &[
                Candidate::new(actor, Reason::Assigned),
                Candidate::new(bystander, Reason::Assigned),
            ],
        );
        assert_eq!(deliveries.len(), 1);
        assert_eq!(deliveries[0].user(), bystander);
    }

    #[test]
    fn self_suppression_beats_even_the_highest_reason() {
        // Assigning a task to yourself and mentioning yourself in the same
        // breath is a real thing people do. Neither is news to them.
        let actor = user();
        let deliveries = resolve(
            Some(actor),
            &[
                Candidate::new(actor, Reason::Mentioned),
                Candidate::new(actor, Reason::Assigned),
                Candidate::new(actor, Reason::Reported),
            ],
        );
        assert!(deliveries.is_empty(), "the actor notified themselves");
    }

    #[test]
    fn four_applicable_reasons_produce_one_notification_at_the_highest() {
        // docs/29 §Acceptance gates, the dedup test, stated there as: "a user
        // with four applicable reasons for one event receives exactly one
        // notification, at the highest rank".
        let recipient = user();
        let deliveries = resolve(
            Some(user()),
            &[
                Candidate::new(recipient, Reason::Team),
                Candidate::new(recipient, Reason::Participated),
                Candidate::new(recipient, Reason::Reported),
                Candidate::new(recipient, Reason::Mentioned),
            ],
        );
        assert_eq!(deliveries.len(), 1);
        assert_eq!(deliveries[0].reason(), Reason::Mentioned);
    }

    #[test]
    fn a_system_event_suppresses_nobody() {
        // `actor_id` is NULL for a system-generated event. Treating that as
        // "the actor is everybody" would silence the notification entirely;
        // treating it as "suppress nobody" is what it means.
        let a = user();
        let b = user();
        let deliveries = resolve(
            None,
            &[
                Candidate::new(a, Reason::Assigned),
                Candidate::new(b, Reason::Reported),
            ],
        );
        assert_eq!(deliveries.len(), 2);
    }

    #[test]
    fn each_recipient_is_resolved_independently() {
        // One person's highest reason must not become everyone's.
        let mentioned = user();
        let assignee = user();
        let deliveries = resolve(
            Some(user()),
            &[
                Candidate::new(mentioned, Reason::Mentioned),
                Candidate::new(assignee, Reason::Assigned),
                Candidate::new(assignee, Reason::Participated),
            ],
        );
        let by_user: BTreeMap<UserId, Reason> =
            deliveries.iter().map(|d| (d.user(), d.reason())).collect();
        assert_eq!(by_user[&mentioned], Reason::Mentioned);
        assert_eq!(by_user[&assignee], Reason::Assigned);
    }

    #[test]
    fn no_candidates_is_no_deliveries() {
        assert!(resolve(Some(user()), &[]).is_empty());
        assert!(resolve(None, &[]).is_empty());
    }

    #[test]
    fn the_same_reason_twice_is_still_one_notification() {
        // Two assignees added in one request, or a candidate query that
        // legitimately returns a person twice.
        let recipient = user();
        let deliveries = resolve(
            Some(user()),
            &[
                Candidate::new(recipient, Reason::Assigned),
                Candidate::new(recipient, Reason::Assigned),
            ],
        );
        assert_eq!(deliveries.len(), 1);
    }

    #[test]
    fn the_result_order_does_not_depend_on_the_candidate_order() {
        // The fan-out writes rows in this order and a test asserts them. A
        // result that depended on however the candidate queries happened to
        // return would be a flake nobody could reproduce.
        let (a, b, c) = (user(), user(), user());
        let forwards = resolve(
            None,
            &[
                Candidate::new(a, Reason::Assigned),
                Candidate::new(b, Reason::Reported),
                Candidate::new(c, Reason::Team),
            ],
        );
        let backwards = resolve(
            None,
            &[
                Candidate::new(c, Reason::Team),
                Candidate::new(b, Reason::Reported),
                Candidate::new(a, Reason::Assigned),
            ],
        );
        assert_eq!(forwards, backwards);
    }

    #[test]
    fn only_ranks_one_to_three_carry_an_immediate_email() {
        let recipient = user();
        for reason in Reason::ALL {
            let deliveries = resolve(None, &[Candidate::new(recipient, reason)]);
            assert_eq!(
                deliveries[0].emails_immediately(),
                reason.rank() <= 3,
                "{reason:?}"
            );
        }
    }
}
