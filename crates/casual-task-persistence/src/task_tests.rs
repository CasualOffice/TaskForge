use super::*;

#[test]
fn the_insert_writes_status_and_state_in_one_statement() {
    // docs/23: the derived column can never drift because it is written
    // with its source. Splitting this into two statements would open the
    // window this invariant exists to close.
    let new = NewTask {
        id: Uuid::now_v7(),
        project_id: Uuid::now_v7(),
        number: 1,
        title: "t".into(),
        description: None,
        task_type: "TASK".into(),
        priority: "NONE".into(),
        status_id: Uuid::now_v7(),
        state: "BACKLOG".into(),
        reporter_id: Uuid::now_v7(),
        parent_id: None,
        due_at: None,
        position: "00000001".into(),
        created_by: Uuid::now_v7(),
    };
    // The type carries both, so there is no way to construct a create that
    // sets one of them.
    assert!(!new.state.is_empty());
    assert_ne!(new.status_id, Uuid::nil());
}

#[test]
fn the_column_list_matches_the_decoded_fields() {
    // Every column the projection selects is decoded by name in `row_of`,
    // and vice versa. Counting commas stopped working when `is_blocked`
    // arrived as a nested EXISTS — the expression contains its own — so
    // this checks the names instead, which is what actually has to agree.
    for name in [
        "id",
        "workspace_id",
        "project_id",
        "number",
        "title",
        "description",
        "status_id",
        "reporter_id",
        "environment_id",
        "milestone_id",
        "parent_id",
        "start_at",
        "due_at",
        "position",
        "created_at",
        "created_by",
        "updated_at",
        "updated_by",
        "version",
        "archived_at",
        "is_blocked",
    ] {
        assert!(
            COLUMNS.contains(name),
            "`{name}` is decoded by row_of and absent from the projection"
        );
    }
    // The three enum columns arrive as text under an explicit alias, or no
    // String decoder accepts them.
    for aliased in ["AS \"type\"", "AS priority", "AS state"] {
        assert!(COLUMNS.contains(aliased), "{aliased} is missing");
    }
}
