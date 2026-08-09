//! Bounds and failure modes for contributions that the host waits on.
//!
//! # The failure this prevents
//!
//! A broken integration stopping a team from working. `docs/34`
//! §`validation.transition` fixes the numbers — 500 ms, no retry, fail-open by
//! default, opt-in fail-closed, breaker at 5 consecutive failures for 60 s —
//! and ADR-017 fixes the default. Those numbers live here as types rather than
//! as constants sprinkled through call sites, because a timeout that differs by
//! call site is a timeout nobody can reason about during an incident.

use core::time::Duration;

use crate::point::ExtensionPoint;

/// What the host does when a contribution times out or errors.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum OnFailure {
    /// Allow the action and record `plugin.validation.skipped` (ADR-017).
    #[default]
    Open,
    /// Refuse the action. Only reachable by an explicit workspace-admin
    /// opt-in, per plugin, with the outage consequence stated at the time.
    Closed,
}

/// The bound the host applies to one synchronous contribution.
///
/// Constructed from the point, never by hand: a call site that could pick its
/// own timeout would eventually pick a different one from the document.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Bounds {
    timeout: Duration,
    on_failure: OnFailure,
    breaker: Breaker,
}

/// The circuit breaker, per `docs/34` §`validation.transition`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Breaker {
    /// Consecutive failures that trip it.
    pub trips_after: u32,
    /// How long it stays open once tripped.
    pub stays_open: Duration,
}

impl Bounds {
    /// `docs/34`: 500 ms for synchronous points, non-negotiable, no retry.
    pub const SYNCHRONOUS_TIMEOUT: Duration = Duration::from_millis(500);
    /// `docs/34`: 10 s for asynchronous delivery.
    pub const ASYNCHRONOUS_TIMEOUT: Duration = Duration::from_secs(10);

    pub const BREAKER: Breaker = Breaker {
        trips_after: 5,
        stays_open: Duration::from_secs(60),
    };

    /// The default bound for a point.
    ///
    /// Asynchronous points get the longer timeout because nobody is waiting;
    /// everything a person waits on gets 500 ms.
    #[must_use]
    pub const fn for_point(point: ExtensionPoint) -> Self {
        use crate::point::Invocation;
        let timeout = match point.invocation() {
            Invocation::OnDomainEvent => Self::ASYNCHRONOUS_TIMEOUT,
            _ => Self::SYNCHRONOUS_TIMEOUT,
        };
        Self {
            timeout,
            on_failure: OnFailure::Open,
            breaker: Self::BREAKER,
        }
    }

    /// A workspace admin's explicit opt-in to fail-closed.
    ///
    /// Named for what it costs, not for what it sets. It takes `self` and
    /// returns a new value so the opt-in is visible at the call site rather
    /// than being a mutation somewhere up the stack.
    #[must_use]
    pub const fn failing_closed_at_the_cost_of_blocking_work(mut self) -> Self {
        self.on_failure = OnFailure::Closed;
        self
    }

    #[must_use]
    pub const fn timeout(self) -> Duration {
        self.timeout
    }

    #[must_use]
    pub const fn on_failure(self) -> OnFailure {
        self.on_failure
    }

    #[must_use]
    pub const fn breaker(self) -> Breaker {
        self.breaker
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_point_defaults_to_fail_open() {
        // ADR-017. If this ever inverts, a plugin outage stops every team.
        for point in ExtensionPoint::ALL {
            assert_eq!(
                Bounds::for_point(*point).on_failure(),
                OnFailure::Open,
                "{point} must fail open by default (ADR-017)"
            );
        }
    }

    #[test]
    fn anything_a_person_waits_on_is_bounded_at_500ms() {
        for point in ExtensionPoint::ALL {
            let bounds = Bounds::for_point(*point);
            if point.invocation() == crate::point::Invocation::OnDomainEvent {
                assert_eq!(bounds.timeout(), Bounds::ASYNCHRONOUS_TIMEOUT);
            } else {
                assert_eq!(
                    bounds.timeout(),
                    Bounds::SYNCHRONOUS_TIMEOUT,
                    "{point} is invoked with a person waiting"
                );
            }
        }
    }

    #[test]
    fn the_blocking_point_is_bounded_the_same_as_the_rest() {
        // The one point that can refuse a user's action gets no extra rope:
        // "it needs longer because it does more" is how a 500 ms budget
        // becomes a 5 s one.
        let bounds = Bounds::for_point(ExtensionPoint::ValidationTransition);
        assert_eq!(bounds.timeout(), Bounds::SYNCHRONOUS_TIMEOUT);
        assert_eq!(bounds.breaker(), Bounds::BREAKER);
    }

    #[test]
    fn fail_closed_is_reachable_only_by_naming_its_cost() {
        let opted_in = Bounds::for_point(ExtensionPoint::ValidationTransition)
            .failing_closed_at_the_cost_of_blocking_work();
        assert_eq!(opted_in.on_failure(), OnFailure::Closed);
        // And it does not leak into the default.
        assert_eq!(
            Bounds::for_point(ExtensionPoint::ValidationTransition).on_failure(),
            OnFailure::Open
        );
    }

    #[test]
    fn the_breaker_matches_the_design_record() {
        assert_eq!(Bounds::BREAKER.trips_after, 5);
        assert_eq!(Bounds::BREAKER.stays_open, Duration::from_secs(60));
    }
}
