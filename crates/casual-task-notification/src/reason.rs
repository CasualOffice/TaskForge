//! Why a person is being told (`docs/29` §Reasons, not events).
//!
//! # The failure this module prevents
//!
//! Three notifications for one event. `docs/29`: "Being mentioned on a task you
//! also reported and commented on yields one notification, labelled
//! `MENTIONED` — not three. This alone removes most of the noise that makes
//! people mute trackers."
//!
//! The mechanism is a **total order** over a closed set. [`Reason::rank`] is an
//! exhaustive match, so a seventh reason cannot be added without deciding where
//! it sits, and [`highest`] is the only way anything in this system turns a set
//! of applicable reasons into the one that is recorded.
//!
//! # Why this is an enum and not a string
//!
//! `notification.reason` is `text` in the schema, so the database would accept
//! anything. A reason that no rank knows about would sort arbitrarily against
//! the others and silently win or lose. The enum is the closed set the column
//! cannot express; [`Reason::as_str`] is the one place the two meet.

use serde::{Deserialize, Serialize};

/// Why a recipient is being notified. Ranked; see the module docs.
///
/// The order of the variants **is** the rank order, and
/// `#[derive(PartialOrd, Ord)]` follows it — declaration order is load-bearing
/// rather than incidental, which is why the ranks below are asserted against it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "UPPERCASE")]
pub enum Reason {
    /// `@user` in a comment or description. Rank 1 — a direct address.
    Mentioned,
    /// Assigned to, or unassigned from, a task.
    Assigned,
    /// A task you filed changed materially.
    Reported,
    /// You explicitly followed the task.
    Subscribed,
    /// You commented on or edited it before.
    Participated,
    /// A team-level rule matched.
    Team,
}

impl Reason {
    /// Every reason, highest first.
    pub const ALL: [Self; 6] = [
        Self::Mentioned,
        Self::Assigned,
        Self::Reported,
        Self::Subscribed,
        Self::Participated,
        Self::Team,
    ];

    /// The rank `docs/29` fixes. 1 is highest.
    ///
    /// Exhaustive on purpose: a seventh reason does not compile until somebody
    /// decides where it sits, which is the decision that matters.
    #[must_use]
    pub const fn rank(self) -> u8 {
        match self {
            Self::Mentioned => 1,
            Self::Assigned => 2,
            Self::Reported => 3,
            Self::Subscribed => 4,
            Self::Participated => 5,
            Self::Team => 6,
        }
    }

    /// The value stored in `notification.reason`.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Mentioned => "MENTIONED",
            Self::Assigned => "ASSIGNED",
            Self::Reported => "REPORTED",
            Self::Subscribed => "SUBSCRIBED",
            Self::Participated => "PARTICIPATED",
            Self::Team => "TEAM",
        }
    }

    /// Parse a stored value. `None` for anything the closed set does not
    /// contain — a row written by a future version, or by hand.
    #[must_use]
    pub fn parse(value: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|r| r.as_str() == value)
    }

    /// Whether this reason sends an immediate email by default.
    ///
    /// `docs/29` §Channels: email is "on for rank 1–3". That is the *default*;
    /// the per-user preference table that would override it does not exist yet
    /// (D-058), so this is the whole policy today rather than its fallback.
    #[must_use]
    pub const fn emails_immediately(self) -> bool {
        self.rank() <= 3
    }
}

/// The one reason a recipient is notified under, out of everything that applies.
///
/// `None` when nothing applies — which is not the same as "notify with no
/// reason", and is why this returns an `Option` rather than a default.
#[must_use]
pub fn highest(applicable: &[Reason]) -> Option<Reason> {
    // `min` and not `max`: rank 1 is the highest reason, and `Ord` follows
    // declaration order, so the *smallest* variant is the one that wins. This
    // is the single line the whole "one notification, not three" property rests
    // on, which is why it is a named function and not an inline fold.
    applicable.iter().copied().min()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_ranks_are_the_ones_docs_29_fixes() {
        // The table in docs/29 §Reasons, not events, transcribed. If a rank
        // moves, the reason a user is shown for an event moves with it.
        assert_eq!(Reason::Mentioned.rank(), 1);
        assert_eq!(Reason::Assigned.rank(), 2);
        assert_eq!(Reason::Reported.rank(), 3);
        assert_eq!(Reason::Subscribed.rank(), 4);
        assert_eq!(Reason::Participated.rank(), 5);
        assert_eq!(Reason::Team.rank(), 6);
    }

    #[test]
    fn declaration_order_and_rank_order_agree() {
        // `highest` uses `Ord`, which follows declaration order. If the two ever
        // disagreed, the wrong reason would win silently and no other test here
        // would notice.
        for pair in Reason::ALL.windows(2) {
            assert!(
                pair[0] < pair[1],
                "{:?} must sort before {:?}",
                pair[0],
                pair[1]
            );
            assert!(pair[0].rank() < pair[1].rank());
        }
    }

    #[test]
    fn every_rank_is_distinct() {
        let mut ranks: Vec<u8> = Reason::ALL.iter().map(|r| r.rank()).collect();
        ranks.sort_unstable();
        ranks.dedup();
        assert_eq!(ranks.len(), Reason::ALL.len(), "two reasons share a rank");
    }

    #[test]
    fn the_documented_example_yields_one_reason_and_it_is_the_mention() {
        // docs/29's own worked example: "Being mentioned on a task you also
        // reported and commented on yields one notification, labelled
        // MENTIONED — not three."
        let applicable = [Reason::Reported, Reason::Participated, Reason::Mentioned];
        assert_eq!(highest(&applicable), Some(Reason::Mentioned));
    }

    #[test]
    fn nothing_applicable_is_not_a_notification() {
        // `None`, not a default. A recipient with no reason is not a recipient,
        // and returning some fallback here would notify them anyway.
        assert_eq!(highest(&[]), None);
    }

    #[test]
    fn the_highest_reason_wins_from_every_starting_order() {
        // `min` over an unsorted slice — the input order is whatever the
        // candidate queries happened to return, and must not matter.
        let mut applicable = vec![Reason::Team, Reason::Assigned, Reason::Participated];
        assert_eq!(highest(&applicable), Some(Reason::Assigned));
        applicable.reverse();
        assert_eq!(highest(&applicable), Some(Reason::Assigned));
    }

    #[test]
    fn email_defaults_cover_exactly_ranks_one_to_three() {
        // docs/29 §Channels: email default is "on for rank 1–3".
        for reason in Reason::ALL {
            assert_eq!(
                reason.emails_immediately(),
                reason.rank() <= 3,
                "{reason:?} disagrees with the documented email default"
            );
        }
    }

    #[test]
    fn the_stored_spelling_round_trips_and_refuses_anything_else() {
        for reason in Reason::ALL {
            assert_eq!(Reason::parse(reason.as_str()), Some(reason));
        }
        assert_eq!(Reason::parse("WATCHED"), None);
        assert_eq!(
            Reason::parse("mentioned"),
            None,
            "the column stores uppercase"
        );
    }

    #[test]
    fn the_wire_format_matches_the_stored_format() {
        // The reason appears in the notification body over HTTP and in the
        // `reason` column. Two spellings would make a client's filter disagree
        // with the database's.
        for reason in Reason::ALL {
            let json = serde_json::to_string(&reason).expect("serializable");
            assert_eq!(json, format!("\"{}\"", reason.as_str()));
        }
    }
}
