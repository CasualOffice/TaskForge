use super::*;
use casual_task_authz::DenyReason;

fn stored(scope_type: &str, id: uuid::Uuid, permission: &str) -> StoredGrant {
    StoredGrant {
        scope_type: scope_type.to_owned(),
        scope_id: id,
        constraints: serde_json::json!({}),
        permission: permission.to_owned(),
    }
}

fn constrained(
    scope_type: &str,
    id: uuid::Uuid,
    permission: &str,
    constraints: serde_json::Value,
) -> StoredGrant {
    StoredGrant {
        scope_type: scope_type.to_owned(),
        scope_id: id,
        constraints,
        permission: permission.to_owned(),
    }
}

#[test]
fn an_unconstrained_grant_reaches_unconditionally() {
    let (actor, workspace) = (UserId::new(), WorkspaceId::new());
    let authority = Authority::resolved(
        actor,
        workspace,
        Vec::new(),
        false,
        &[stored("WORKSPACE", workspace.as_uuid(), "task.close")],
    );
    assert_eq!(
        authority.effective_in_workspace(),
        vec![Effective {
            permission: permission::TASK_CLOSE,
            reach: Reach::Unconditional
        }]
    );
}

#[test]
fn a_constrained_grant_is_reported_as_conditional_not_dropped() {
    // The failure this prevents: evaluating "you may close tasks you are
    // assigned to" against empty facts and reporting "you may not close
    // tasks" — a feature the actor holds and never sees.
    let (actor, workspace) = (UserId::new(), WorkspaceId::new());
    let authority = Authority::resolved(
        actor,
        workspace,
        Vec::new(),
        false,
        &[constrained(
            "WORKSPACE",
            workspace.as_uuid(),
            "task.close",
            serde_json::json!({ "assignee_is_actor": true }),
        )],
    );
    assert_eq!(
        authority.effective_in_workspace(),
        vec![Effective {
            permission: permission::TASK_CLOSE,
            reach: Reach::Conditional
        }]
    );
}

#[test]
fn an_unconstrained_grant_beats_a_constrained_one_for_the_same_permission() {
    // The resolver's own combining rule: an unconstrained grant wins
    // outright. Reporting Conditional here would understate authority and
    // hide a control the actor can always use.
    let (actor, workspace) = (UserId::new(), WorkspaceId::new());
    let authority = Authority::resolved(
        actor,
        workspace,
        Vec::new(),
        false,
        &[
            constrained(
                "WORKSPACE",
                workspace.as_uuid(),
                "task.close",
                serde_json::json!({ "assignee_is_actor": true }),
            ),
            stored("WORKSPACE", workspace.as_uuid(), "task.close"),
        ],
    );
    assert_eq!(
        authority.effective_in_workspace(),
        vec![Effective {
            permission: permission::TASK_CLOSE,
            reach: Reach::Unconditional
        }],
        "reported once, and unconditionally"
    );
}

#[test]
fn the_effective_set_never_repeats_a_permission() {
    let (actor, workspace) = (UserId::new(), WorkspaceId::new());
    let authority = Authority::resolved(
        actor,
        workspace,
        Vec::new(),
        false,
        &[
            stored("WORKSPACE", workspace.as_uuid(), "task.read"),
            stored("WORKSPACE", workspace.as_uuid(), "task.read"),
        ],
    );
    assert_eq!(authority.effective_in_workspace().len(), 1);
}

#[test]
fn explaining_a_permission_nobody_granted_says_no_grant_and_lists_nothing() {
    let (actor, workspace) = (UserId::new(), WorkspaceId::new());
    let authority = Authority::resolved(actor, workspace, Vec::new(), false, &[]);
    let explanation = authority.explain_in_workspace(permission::TASK_CLOSE);
    assert!(!explanation.allowed);
    assert_eq!(explanation.deny_reason, Some("no_grant"));
    assert!(explanation.contributing.is_empty());
}

#[test]
fn a_grant_can_contribute_and_still_not_allow() {
    // This is the whole point of `/explain`. "You have task.close through a
    // workspace grant, but it requires you to be the assignee and you are
    // not" is a useful answer; "no" is not.
    let (actor, workspace, project) = (UserId::new(), WorkspaceId::new(), ProjectId::new());
    let authority = Authority::resolved(
        actor,
        workspace,
        Vec::new(),
        false,
        &[constrained(
            "WORKSPACE",
            workspace.as_uuid(),
            "task.close",
            serde_json::json!({ "assignee_is_actor": true }),
        )],
    );
    let explanation = authority.explain_in_project(
        permission::TASK_CLOSE,
        project,
        &[],
        &ResourceFacts::default(),
    );
    assert!(!explanation.allowed);
    assert_eq!(explanation.deny_reason, Some("constraint_unsatisfied"));
    assert_eq!(explanation.contributing.len(), 1, "the grant is named");
    let grant = &explanation.contributing[0];
    assert_eq!(grant.scope_type, "WORKSPACE");
    assert_eq!(grant.scope_id, workspace.as_uuid());
    assert_eq!(grant.constraints, vec!["assignee_is_actor"]);
    assert!(!grant.constraints_satisfied);
}

#[test]
fn the_same_grant_satisfies_once_the_facts_hold() {
    let (actor, workspace, project) = (UserId::new(), WorkspaceId::new(), ProjectId::new());
    let authority = Authority::resolved(
        actor,
        workspace,
        Vec::new(),
        false,
        &[constrained(
            "WORKSPACE",
            workspace.as_uuid(),
            "task.close",
            serde_json::json!({ "assignee_is_actor": true }),
        )],
    );
    let facts = ResourceFacts {
        assignees: vec![actor],
        ..ResourceFacts::default()
    };
    let explanation = authority.explain_in_project(permission::TASK_CLOSE, project, &[], &facts);
    assert!(explanation.allowed);
    assert_eq!(explanation.deny_reason, None);
    assert!(explanation.contributing[0].constraints_satisfied);
}

#[test]
fn an_explanation_never_names_a_grant_from_another_workspace() {
    // `applicable` filters on workspace before anything else, and this
    // asserts the explanation inherits that rather than re-deriving it.
    let (actor, workspace, elsewhere) = (UserId::new(), WorkspaceId::new(), WorkspaceId::new());
    let authority = Authority::resolved(
        actor,
        workspace,
        Vec::new(),
        false,
        &[stored("WORKSPACE", elsewhere.as_uuid(), "task.close")],
    );
    let explanation = authority.explain_in_workspace(permission::TASK_CLOSE);
    assert!(explanation.contributing.is_empty());
    assert_eq!(explanation.deny_reason, Some("no_grant"));
}

#[test]
fn an_actor_with_no_grants_may_do_nothing() {
    // The direction that matters. A workspace with no role assignment must
    // not confer authority by virtue of membership — migration 0003:
    // "role_assignment is the ONLY source of authority in the system".
    let authority = Authority::resolved(UserId::new(), WorkspaceId::new(), Vec::new(), false, &[]);
    assert_eq!(
        authority.may_in_workspace(permission::PROJECT_CREATE),
        Decision::Deny(DenyReason::NoGrant)
    );
}

#[test]
fn a_workspace_grant_reaches_a_project_in_that_workspace() {
    let (actor, workspace) = (UserId::new(), WorkspaceId::new());
    let authority = Authority::resolved(
        actor,
        workspace,
        Vec::new(),
        false,
        &[stored("WORKSPACE", workspace.as_uuid(), "task.create")],
    );
    assert!(
        authority
            .may_in_project(
                permission::TASK_CREATE,
                ProjectId::new(),
                &[],
                &ResourceFacts::default()
            )
            .is_allowed()
    );
}

#[test]
fn a_grant_from_another_workspace_never_contributes() {
    // The cross-tenant case. The resolver filters on workspace, and this is
    // the mapping that has to keep the workspace on the grant.
    let (actor, workspace) = (UserId::new(), WorkspaceId::new());
    let authority = Authority::resolved(
        actor,
        workspace,
        Vec::new(),
        false,
        &[stored(
            "WORKSPACE",
            WorkspaceId::new().as_uuid(),
            "project.create",
        )],
    );
    assert_eq!(
        authority.may_in_workspace(permission::PROJECT_CREATE),
        Decision::Deny(DenyReason::NoGrant)
    );
}

#[test]
fn a_project_grant_does_not_reach_a_sibling_project() {
    let (actor, workspace, project) = (UserId::new(), WorkspaceId::new(), ProjectId::new());
    let authority = Authority::resolved(
        actor,
        workspace,
        Vec::new(),
        false,
        &[stored("PROJECT", project.as_uuid(), "task.create")],
    );
    assert!(
        authority
            .may_in_project(
                permission::TASK_CREATE,
                project,
                &[],
                &ResourceFacts::default()
            )
            .is_allowed()
    );
    assert!(
        !authority
            .may_in_project(
                permission::TASK_CREATE,
                ProjectId::new(),
                &[],
                &ResourceFacts::default()
            )
            .is_allowed()
    );
}

#[test]
fn an_unreadable_constraint_narrows_the_grant_to_nothing() {
    // The failure direction. Dropping a constraint we cannot parse would
    // widen the grant to unconstrained, which hands out authority nobody
    // granted.
    let (actor, workspace) = (UserId::new(), WorkspaceId::new());
    let mut row = stored("WORKSPACE", workspace.as_uuid(), "project.create");
    row.constraints = serde_json::json!({ "invented_by_a_future_version": true });
    let authority = Authority::resolved(actor, workspace, Vec::new(), false, &[row]);
    assert_eq!(
        authority.may_in_workspace(permission::PROJECT_CREATE),
        Decision::Deny(DenyReason::ConstraintUnsatisfied)
    );
}

#[test]
fn a_documented_constraint_is_understood() {
    // The other half: `not_external` must actually work, or the test above
    // would pass with a parser that understands nothing.
    let (actor, workspace) = (UserId::new(), WorkspaceId::new());
    let mut row = stored("WORKSPACE", workspace.as_uuid(), "project.create");
    row.constraints = serde_json::json!({ "not_external": true });
    let authority = Authority::resolved(actor, workspace, Vec::new(), false, &[row]);
    assert!(
        authority
            .may_in_workspace(permission::PROJECT_CREATE)
            .is_allowed()
    );
}

#[test]
fn an_unknown_permission_key_removes_authority_rather_than_adding_it() {
    let (actor, workspace) = (UserId::new(), WorkspaceId::new());
    let authority = Authority::resolved(
        actor,
        workspace,
        Vec::new(),
        false,
        &[stored("WORKSPACE", workspace.as_uuid(), "project.invent")],
    );
    assert_eq!(
        authority.may_in_workspace(permission::PROJECT_CREATE),
        Decision::Deny(DenyReason::NoGrant)
    );
}

#[test]
fn project_scoped_grants_are_collected_for_the_visibility_clause() {
    let (actor, workspace) = (UserId::new(), WorkspaceId::new());
    let (a, b) = (ProjectId::new(), ProjectId::new());
    let authority = Authority::resolved(
        actor,
        workspace,
        Vec::new(),
        false,
        &[
            stored("PROJECT", a.as_uuid(), "task.read"),
            stored("PROJECT", a.as_uuid(), "task.create"),
            stored("PROJECT", b.as_uuid(), "task.read"),
            stored("WORKSPACE", workspace.as_uuid(), "task.read"),
        ],
    );
    let mut expected = vec![a, b];
    expected.sort_unstable();
    assert_eq!(
        authority.granted_projects(),
        expected,
        "workspace-scoped grants must not appear here: docs/04 confers \
             visibility from a grant scoped to the project, and widening it \
             would make every private project visible to every member"
    );
}

#[test]
fn a_task_type_constraint_decodes_to_the_types_it_names() {
    // The regression this pins: `task_type_in` had no arm here, so every
    // grant carrying it fell through to `unsatisfiable` and denied its
    // holder everything. Nothing failed loudly — a rule that denies looks
    // like a strict administrator, not like a bug.
    let decoded = constraints_of(&serde_json::json!({ "task_type_in": ["BUG", "INCIDENT"] }));
    assert_eq!(
        decoded,
        vec![Constraint::TaskTypeIn(vec![
            casual_task_model::TaskType::Bug,
            casual_task_model::TaskType::Incident
        ])]
    );
}

#[test]
fn an_unknown_type_narrows_the_list_rather_than_breaking_the_grant() {
    // A type this build does not know is a type it cannot be asked to
    // allow. Dropping it leaves a narrower grant; making the whole
    // constraint unsatisfiable would take away the types that *are*
    // spelled correctly beside it.
    let decoded = constraints_of(&serde_json::json!({ "task_type_in": ["BUG", "CHORE"] }));
    assert_eq!(
        decoded,
        vec![Constraint::TaskTypeIn(vec![
            casual_task_model::TaskType::Bug
        ])]
    );
}

#[test]
fn a_task_type_constraint_that_is_not_a_list_is_unsatisfiable() {
    // Malformed, not narrow. Failing closed is right here: nobody can say
    // what `"task_type_in": "BUG"` was meant to permit, and guessing "the
    // one type it names" would turn a typo into a grant.
    assert_eq!(
        constraints_of(&serde_json::json!({ "task_type_in": "BUG" })),
        vec![unsatisfiable()]
    );
}

#[test]
fn an_empty_task_type_list_permits_nothing() {
    // Distinct from an absent constraint, which permits everything. A
    // grant narrowed to no types is a grant that raises nothing, and
    // reading it as "unrestricted" would invert it.
    let decoded = constraints_of(&serde_json::json!({ "task_type_in": [] }));
    assert_eq!(decoded, vec![Constraint::TaskTypeIn(Vec::new())]);
    let facts = ResourceFacts {
        task_type: Some(casual_task_model::TaskType::Bug),
        ..ResourceFacts::default()
    };
    let actor = casual_task_model::UserId::from_uuid(uuid::Uuid::now_v7());
    assert!(!decoded[0].satisfied(actor, &facts));
}
