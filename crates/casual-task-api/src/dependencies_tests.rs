use super::*;

fn body(json: &str) -> AddDependency {
    serde_json::from_str(json).expect("valid")
}

#[test]
fn a_request_must_name_exactly_one_direction() {
    let this = Uuid::now_v7();
    assert_eq!(
        Direction::of(&body("{}"), this, "r")
            .err()
            .map(|e| e.code()),
        Some(codes::MISSING_FIELD)
    );
    let both = format!(
        r#"{{"blocks":"{}","blocked_by":"{}"}}"#,
        Uuid::now_v7(),
        Uuid::now_v7()
    );
    assert_eq!(
        Direction::of(&body(&both), this, "r")
            .err()
            .map(|e| e.code()),
        Some(codes::MALFORMED_BODY)
    );
}

#[test]
fn the_two_directions_produce_opposite_edges() {
    // The bug this catches renders the whole Relations panel backwards and
    // gates the wrong task's transitions — and looks entirely plausible.
    let (this, other) = (Uuid::now_v7(), Uuid::now_v7());
    assert_eq!(Direction::Blocks { this, other }.edge(), (this, other));
    assert_eq!(Direction::BlockedBy { this, other }.edge(), (other, this));
}

#[test]
fn the_edge_direction_matches_the_schemas_column_names() {
    // migration 0005: `from_task_id` blocks `to_task_id`, and
    // `task::unresolved_blockers` reads it that way — it joins blockers on
    // `from_task_id` where `to_task_id` is the task being transitioned.
    let migration = include_str!("../../../migrations/0005_tasks.sql");
    assert!(migration.contains("from_task_id"));
    assert!(migration.contains("to_task_id"));
    let (this, other) = (Uuid::now_v7(), Uuid::now_v7());
    let (from, to) = Direction::Blocks { this, other }.edge();
    assert_eq!((from, to), (this, other), "`this` blocks `other`");
}

#[test]
fn an_unknown_field_does_not_deserialize() {
    // docs/05: unknown request fields are rejected.
    assert!(serde_json::from_str::<AddDependency>(r#"{"blcoks":"x"}"#).is_err());
}

#[test]
fn the_label_names_the_direction_the_caller_asked_for() {
    // It goes into the activity record, which is rendered years later.
    let (this, other) = (Uuid::now_v7(), Uuid::now_v7());
    assert_eq!(Direction::Blocks { this, other }.label(), "blocks");
    assert_eq!(Direction::BlockedBy { this, other }.label(), "blocked_by");
    assert_eq!(Direction::Blocks { this, other }.other(), other);
}
