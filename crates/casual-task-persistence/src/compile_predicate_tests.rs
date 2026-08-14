#[test]
fn blocked_means_the_task_is_the_to_end_of_an_unresolved_edge() {
    let sql = compiled_sql_of(
        Field::IsBlocked,
        Operator::Eq,
        Value::Literal("true".into()),
    );
    assert!(sql.contains("d.to_task_id = t.id"));
    assert!(!sql.contains("d.from_task_id = t.id"));
    assert!(sql.contains("COMPLETED"));
}

fn compiled_sql_of(field: Field, op: Operator, value: Value) -> String {
    compiled(&clause(field, op, value)).sql
}

#[test]
fn key_matches_nothing_rather_than_the_wrong_rows() {
    let compiled = compiled(&clause(
        Field::Key,
        Operator::Eq,
        Value::Literal("WR-125".into()),
    ));
    assert!(compiled.sql.contains("FALSE"));
    assert!(!compiled.sql.contains("WR-125"));
    assert_eq!(compiled.params.len(), 2);
}

#[test]
fn wildcards_are_applied_in_sql_around_a_parameter() {
    let compiled = compiled(&clause(
        Field::Title,
        Operator::Contains,
        Value::Literal("100%".into()),
    ));
    assert!(compiled.sql.contains("LIKE '%' || $3 || '%'"));
    assert!(!compiled.sql.contains("100%"));
}

#[test]
fn age_counts_only_open_work() {
    let compiled = compile_age(
        &Node::And(vec![]),
        WorkspaceId::new(),
        &AuthorizedProjectSet::resolved(vec![ProjectId::new()]),
        Dimension::Project,
        Reduce::Max,
        20,
    );
    assert!(
        compiled
            .sql
            .contains("t.state NOT IN ('COMPLETED', 'CANCELED')")
    );
}

#[test]
fn age_does_not_join_the_state_projection() {
    let compiled = compile_age(
        &Node::And(vec![]),
        WorkspaceId::new(),
        &AuthorizedProjectSet::resolved(vec![ProjectId::new()]),
        Dimension::Project,
        Reduce::Max,
        20,
    );
    assert!(!compiled.sql.contains("task_state_interval"));
}

#[test]
fn age_carries_the_tenant_and_the_authorized_projects() {
    let compiled = compile_age(
        &Node::And(vec![]),
        WorkspaceId::new(),
        &AuthorizedProjectSet::resolved(vec![ProjectId::new()]),
        Dimension::Assignee,
        Reduce::P90,
        20,
    );
    assert!(compiled.sql.contains("t.workspace_id = $1"));
    assert!(compiled.sql.contains("t.project_id = ANY($2)"));
    assert!(compiled.sql.contains("t.deleted_at IS NULL"));
}
