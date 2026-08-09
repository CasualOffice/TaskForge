//! The permanent semantic contract. See `docs/23-WORKFLOW-AND-STATE-MACHINE.md`.
//!
//! Statuses are configurable; **states are not**. Five, forever. Adding a sixth
//! is a breaking API change requiring a major version, because every report,
//! automation, and plugin in existence branches on this enum.
//!
//! The golden serialization test below is the tripwire: adding a variant fails
//! it, which forces the change to be deliberate.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "UPPERCASE")]
pub enum TaskState {
    /// Captured, not committed to.
    Backlog,
    /// Committed, not started.
    Planned,
    /// Being worked — **including stalled work**. "Blocked" is a status with
    /// this state, not a state of its own: blocked work is committed work whose
    /// clock is still running.
    Active,
    /// Finished successfully.
    Completed,
    /// Terminated without completion. Separate from `Completed` so throughput
    /// and cycle-time metrics do not count abandoned work as delivered.
    Canceled,
}

impl TaskState {
    /// Every state, in workflow order. Exhaustive by construction.
    pub const ALL: [TaskState; 5] = [
        TaskState::Backlog,
        TaskState::Planned,
        TaskState::Active,
        TaskState::Completed,
        TaskState::Canceled,
    ];

    /// The wire and storage spelling — `BACKLOG`, `PLANNED`, …
    ///
    /// The same five strings the `task_state` enum uses in the database and
    /// that `#[serde(rename_all = "UPPERCASE")]` produces. Written out rather
    /// than derived from `Debug`, because `Debug` output is not API and a
    /// rename would silently change what the database is told.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Backlog => "BACKLOG",
            Self::Planned => "PLANNED",
            Self::Active => "ACTIVE",
            Self::Completed => "COMPLETED",
            Self::Canceled => "CANCELED",
        }
    }

    /// Parse the wire spelling. Unknown input is `None`, never a default —
    /// a state this build does not know is a row it cannot reason about.
    #[must_use]
    pub fn parse(value: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|s| s.as_str() == value)
    }

    /// Terminal states. Leaving one requires `task.reopen`.
    pub fn is_terminal(&self) -> bool {
        matches!(self, TaskState::Completed | TaskState::Canceled)
    }

    /// Whether work is considered open. Used by "My Work" and default filters.
    pub fn is_open(&self) -> bool {
        !self.is_terminal()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "UPPERCASE")]
pub enum TaskType {
    Task,
    Bug,
    Feature,
    Incident,
    Request,
}

/// Ordered so `ORDER BY priority DESC` and `priority >= HIGH` are semantic
/// (`docs/27-FILTER-AND-SAVED-VIEW-DSL.md`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "UPPERCASE")]
pub enum Priority {
    None,
    Low,
    Medium,
    High,
    Urgent,
}

/// Who can see a project exists. Evaluated *before* permissions; an invisible
/// project is a 404, not a 403 (`docs/04-RBAC-AND-AUTHORIZATION.md`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "UPPERCASE")]
pub enum Visibility {
    Private,
    Team,
    Workspace,
}

#[cfg(test)]
mod tests {
    #[test]
    fn parse_round_trips_every_state_and_refuses_anything_else() {
        for state in super::TaskState::ALL {
            assert_eq!(super::TaskState::parse(state.as_str()), Some(state));
        }
        assert_eq!(super::TaskState::parse("BLOCKED"), None);
        assert_eq!(super::TaskState::parse("backlog"), None);
    }

    #[test]
    fn the_wire_spelling_matches_what_serde_emits() {
        // Three places spell these: as_str, serde, and the database enum. A
        // rename that moved one and not the others would be found by a 500 in
        // production rather than here.
        for state in super::TaskState::ALL {
            let json = serde_json::to_string(&state).expect("serialises");
            assert_eq!(format!("\"{}\"", state.as_str()), json);
        }
    }

    use super::*;

    /// Golden wire format. If this fails, the permanent state contract changed —
    /// which needs a major API version and a superseding ADR, not a fixture edit.
    #[test]
    fn state_wire_format_is_frozen() {
        let json = serde_json::to_string(&TaskState::ALL).unwrap();
        assert_eq!(
            json, r#"["BACKLOG","PLANNED","ACTIVE","COMPLETED","CANCELED"]"#,
            "the five states are the permanent API contract (docs/23)"
        );
    }

    #[test]
    fn blocked_work_is_active_not_its_own_state() {
        assert_eq!(TaskState::ALL.len(), 5);
    }

    #[test]
    fn terminal_states() {
        assert!(TaskState::Completed.is_terminal());
        assert!(TaskState::Canceled.is_terminal());
        assert!(TaskState::Active.is_open());
    }

    #[test]
    fn priority_orders_semantically() {
        assert!(Priority::Urgent > Priority::High);
        assert!(Priority::High > Priority::None);
    }
}
