//! Enum labels, as PostgreSQL spells them.
//!
//! The mapping is written out rather than derived from `serde`, because the
//! database labels (migration 0001) and the wire format
//! (`casual-task-model::state`) are two separate contracts that happen to
//! agree today. Deriving one from the other would hide the day they stop
//! agreeing; the golden tests below fail on it instead.

use casual_task_model::{Priority, TaskState, TaskType, Visibility};

pub fn state(s: TaskState) -> &'static str {
    match s {
        TaskState::Backlog => "BACKLOG",
        TaskState::Planned => "PLANNED",
        TaskState::Active => "ACTIVE",
        TaskState::Completed => "COMPLETED",
        TaskState::Canceled => "CANCELED",
    }
}

pub fn state_index(s: TaskState) -> usize {
    match s {
        TaskState::Backlog => 0,
        TaskState::Planned => 1,
        TaskState::Active => 2,
        TaskState::Completed => 3,
        TaskState::Canceled => 4,
    }
}

pub fn task_type(t: TaskType) -> &'static str {
    match t {
        TaskType::Task => "TASK",
        TaskType::Bug => "BUG",
        TaskType::Feature => "FEATURE",
        TaskType::Incident => "INCIDENT",
        TaskType::Request => "REQUEST",
    }
}

pub fn priority(p: Priority) -> &'static str {
    match p {
        Priority::None => "NONE",
        Priority::Low => "LOW",
        Priority::Medium => "MEDIUM",
        Priority::High => "HIGH",
        Priority::Urgent => "URGENT",
    }
}

pub fn visibility(v: Visibility) -> &'static str {
    match v {
        Visibility::Private => "PRIVATE",
        Visibility::Team => "TEAM",
        Visibility::Workspace => "WORKSPACE",
    }
}

/// The five task types, in the order migration 0001 declares them.
pub const TASK_TYPES: [TaskType; 5] = [
    TaskType::Task,
    TaskType::Bug,
    TaskType::Feature,
    TaskType::Incident,
    TaskType::Request,
];

pub const PRIORITIES: [Priority; 5] = [
    Priority::None,
    Priority::Low,
    Priority::Medium,
    Priority::High,
    Priority::Urgent,
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn state_labels_match_the_enum_declaration_order() {
        let labels: Vec<&str> = TaskState::ALL.iter().map(|s| state(*s)).collect();
        assert_eq!(
            labels,
            ["BACKLOG", "PLANNED", "ACTIVE", "COMPLETED", "CANCELED"],
            "must match CREATE TYPE task_state in migration 0001"
        );
    }

    #[test]
    fn state_index_is_dense_and_ordered() {
        for (i, s) in TaskState::ALL.iter().enumerate() {
            assert_eq!(state_index(*s), i);
        }
    }

    #[test]
    fn type_and_priority_labels_match_migration_0001() {
        let types: Vec<&str> = TASK_TYPES.iter().map(|t| task_type(*t)).collect();
        assert_eq!(types, ["TASK", "BUG", "FEATURE", "INCIDENT", "REQUEST"]);
        let prios: Vec<&str> = PRIORITIES.iter().map(|p| priority(*p)).collect();
        assert_eq!(prios, ["NONE", "LOW", "MEDIUM", "HIGH", "URGENT"]);
    }
}
