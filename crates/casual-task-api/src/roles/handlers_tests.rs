use super::*;

#[test]
fn every_registered_permission_parses_and_a_typo_does_not() {
    for p in permission::ALL {
        assert_eq!(
            parse_permissions(&[p.as_str().to_owned()], "r").expect("known"),
            vec![*p]
        );
    }
    assert!(parse_permissions(&["task.updat".to_owned()], "r").is_err());
}

#[test]
fn every_scope_type_the_model_has_parses_and_task_scope_does_not() {
    for scope_type in ["WORKSPACE", "TEAM", "PROJECT", "ENVIRONMENT"] {
        assert!(parse_scope(scope_type, Uuid::now_v7(), "r").is_ok());
    }
    // ADR-005 excludes it, and the enum has no member for it.
    assert!(parse_scope("TASK", Uuid::now_v7(), "r").is_err());
}

#[test]
fn each_refusal_maps_to_the_code_that_names_its_rule() {
    // `docs/04` gives every control its own number, and `docs/20` a code.
    // Collapsing two onto one would tell an admin they hit "a rule".
    let cases = [
        (
            Refusal::ExceedsGrantCeiling {
                missing: permission::WORKSPACE_DELETE,
            },
            "TF-AZN-0003",
        ),
        (Refusal::ExceedsScopeCeiling, "TF-AZN-0004"),
        (Refusal::RoleEditingIsWorkspaceScoped, "TF-AZN-0004"),
        (
            Refusal::SelfElevation {
                missing: permission::WORKSPACE_OWNER,
            },
            "TF-AZN-0006",
        ),
    ];
    for (refusal, expected) in cases {
        assert_eq!(refused(&refusal, "r").code().as_str(), expected);
    }
}
