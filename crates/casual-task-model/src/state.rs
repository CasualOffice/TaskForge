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
