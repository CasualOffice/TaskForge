use super::*;

#[test]
fn the_visibility_predicate_matches_the_four_documented_routes() {
    // docs/04 §Visibility vs permission lists exactly four ways in. A route
    // silently dropped here becomes a project somebody can no longer see,
    // and nothing else in the system would report it.
    assert!(VISIBLE.contains("p.visibility = 'WORKSPACE'"));
    assert!(VISIBLE.contains("p.visibility = 'TEAM'"));
    assert!(VISIBLE.contains("project_membership"));
    assert!(VISIBLE.contains("p.id = ANY($4)"));
}

#[test]
fn team_visibility_reads_the_join_and_not_the_superseded_column() {
    // Migration 0027 replaced `project.team_id` with `project_team`. A
    // predicate left on the old column would answer for the FIRST team a
    // project ever had and silently hide it from every other one.
    assert!(VISIBLE.contains("project_team"));
    assert!(!VISIBLE.contains("p.team_id"));
    assert!(!COLUMNS.contains("p.team_id"));
}

#[test]
fn no_read_in_this_module_paginates_by_offset() {
    // docs/26 bans it outright and casual-task-lint bans the token; this
    // asserts the shared fragments cannot smuggle one in.
    for sql in [VISIBLE, COLUMNS] {
        assert!(!sql.to_uppercase().contains("OFFSET "), "{sql}");
    }
}
