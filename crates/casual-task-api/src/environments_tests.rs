use super::*;

#[test]
fn an_environment_name_is_bounded_at_both_ends() {
    assert_eq!(validated_name("  staging  ", "r").ok(), Some("staging"));
    for bad in ["", "   "] {
        assert!(validated_name(bad, "r").is_err(), "{bad:?}");
    }
    assert!(validated_name(&"x".repeat(40), "r").is_ok());
    assert!(validated_name(&"x".repeat(41), "r").is_err());
}

#[test]
fn an_unknown_field_does_not_deserialize() {
    assert!(serde_json::from_str::<CreateRequest>(r#"{"nmae":"staging"}"#).is_err());
    assert!(serde_json::from_str::<SetOnTaskRequest>(r#"{"env":null}"#).is_err());
}

#[test]
fn clearing_a_task_environment_is_spelled_and_not_implied() {
    // An absent field must stay DISTINGUISHABLE from an explicit null, so
    // the handler can refuse the first and honour the second.
    assert_eq!(
        serde_json::from_str::<SetOnTaskRequest>("{}")
            .expect("valid json")
            .environment_id,
        None,
        "absent, which the handler refuses with TF-VAL-0003"
    );
    let cleared: SetOnTaskRequest =
        serde_json::from_str(r#"{"environment_id":null}"#).expect("valid");
    assert_eq!(
        cleared.environment_id,
        Some(None),
        "present and null: clear it"
    );
    let set: SetOnTaskRequest =
        serde_json::from_str(r#"{"environment_id":"018f2c00-0000-7000-8000-000000000000"}"#)
            .expect("valid");
    assert!(matches!(set.environment_id, Some(Some(_))));
}
