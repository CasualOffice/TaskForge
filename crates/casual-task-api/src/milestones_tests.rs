use super::*;

#[test]
fn a_name_is_trimmed_bounded_and_required() {
    assert_eq!(validated_name("  1.4  ", "r").ok(), Some("1.4"));
    assert_eq!(
        validated_name("   ", "r").err().map(|e| e.code()),
        Some(codes::MISSING_FIELD)
    );
    let long = "x".repeat(MAX_NAME + 1);
    assert_eq!(
        validated_name(&long, "r").err().map(|e| e.code()),
        Some(codes::OUT_OF_RANGE)
    );
}

#[test]
fn a_due_date_is_rfc_3339_or_a_400() {
    assert!(parse_due(None, "r").expect("absent is fine").is_none());
    assert!(parse_due(Some("2026-09-01T00:00:00Z"), "r").is_ok());
    assert_eq!(
        parse_due(Some("next friday"), "r").err().map(|e| e.code()),
        Some(codes::MALFORMED_BODY)
    );
}

#[test]
fn a_client_cannot_backdate_a_completion() {
    // `completed` is a bool. A `completed_at` field would let a client
    // decide when a milestone closed, which is a number reports read.
    assert!(serde_json::from_str::<PatchRequest>(r#"{"completed":true}"#).is_ok());
    assert!(
        serde_json::from_str::<PatchRequest>(r#"{"completed_at":"2020-01-01T00:00:00Z"}"#).is_err(),
        "completed_at must not be settable"
    );
}

#[test]
fn the_patch_distinguishes_absent_from_cleared() {
    // docs/05 §Conventions. Without `double_option` a client can never
    // remove a due date — the clear collapses into "unchanged" and the
    // request appears to succeed.
    let absent: PatchRequest = serde_json::from_str("{}").expect("valid");
    assert!(absent.due_at.is_none());
    let cleared: PatchRequest = serde_json::from_str(r#"{"due_at":null}"#).expect("valid");
    assert_eq!(cleared.due_at, Some(None));
}

#[test]
fn nothing_in_this_module_writes_a_task() {
    // The rule the module docs open with, held against its own source.
    // `docs/03`: rollup is displayed, never enforced — so a milestone
    // handler that reached into the task repository to move work would be
    // building the behaviour this file exists to refuse.
    //
    // The banned names are assembled rather than written out, and this
    // comment names none of them, because the check reads this very file:
    // spelling one here would fail the test on its own explanation.
    let source = include_str!("milestones.rs");
    for banned in [
        format!("task{}update", "::"),
        format!("task{}transition", "::"),
        format!("task{}soft_delete", "::"),
    ] {
        assert!(
            !source.contains(&banned),
            "milestones.rs calls `{banned}`; closing a milestone must move no task"
        );
    }
}
