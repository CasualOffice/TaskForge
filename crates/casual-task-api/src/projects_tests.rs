use super::*;

#[test]
fn the_key_format_matches_the_check_constraint() {
    // migrations/0004: key ~ '^[A-Z][A-Z0-9]{1,9}$'. If this drifts, a key
    // the API accepts becomes a 500 from a constraint violation.
    for good in ["WR", "OPS", "A1", "ABCDEFGHIJ", "X9Z8Y7"] {
        assert!(well_formed_key(good), "{good} should be accepted");
    }
    for bad in [
        "",
        "A",
        "a",
        "wr",
        "Wr",
        "ABCDEFGHIJK",
        "AB-C",
        "AB C",
        "1AB",
        "AB_C",
        "ÄB",
    ] {
        assert!(!well_formed_key(bad), "{bad} should be refused");
    }
}

#[test]
fn visibility_defaults_to_the_databases_default() {
    // Two defaults that disagree would mean a project created through the
    // API and one created by a migration behave differently.
    assert_eq!(validated_visibility(None, "r").ok(), Some("TEAM"));
    let default_in_migration = include_str!("../../../migrations/0004_projects_and_workflow.sql");
    assert!(
        default_in_migration.contains("visibility    visibility NOT NULL DEFAULT 'TEAM'"),
        "migration 0004 no longer defaults visibility to TEAM"
    );
}

#[test]
fn an_unknown_visibility_is_refused_rather_than_defaulted() {
    assert_eq!(
        validated_visibility(Some("PUBLIC"), "r")
            .err()
            .map(|e| e.code()),
        Some(codes::INVALID_ENUM)
    );
}

#[test]
fn a_name_is_bounded_at_both_ends() {
    assert!(validated_name("Work", "r").is_ok());
    assert_eq!(validated_name("Work", "r").ok(), Some("Work"));
    assert!(validated_name("  Work  ", "r").is_ok());
    for bad in ["", "   "] {
        assert!(validated_name(bad, "r").is_err(), "{bad:?}");
    }
    assert!(validated_name(&"x".repeat(201), "r").is_err());
    assert!(validated_name(&"x".repeat(200), "r").is_ok());
}

#[test]
fn every_visibility_the_enum_declares_is_accepted() {
    let migration = include_str!("../../../migrations/0001_extensions_and_types.sql");
    for value in VISIBILITIES {
        assert!(validated_visibility(Some(value), "r").is_ok());
        assert!(
            migration.contains(&format!("'{value}'")),
            "{value} is not a member of the visibility enum"
        );
    }
}

#[test]
fn a_patch_distinguishes_an_absent_description_from_a_null_one() {
    // docs/05 §Conventions: `PATCH {"description": null}` clears it,
    // `PATCH {}` leaves it. Collapsing them silently wipes a field.
    let absent: PatchRequest = serde_json::from_str("{}").expect("valid");
    assert_eq!(absent.description, None);

    let cleared: PatchRequest = serde_json::from_str(r#"{"description":null}"#).expect("valid");
    assert_eq!(cleared.description, Some(None));

    let set: PatchRequest = serde_json::from_str(r#"{"description":"x"}"#).expect("valid");
    assert_eq!(set.description, Some(Some("x".to_owned())));
}

#[test]
fn an_unknown_field_does_not_deserialize() {
    // docs/05: unknown request fields are rejected. Silently ignoring a
    // typo is how a client ships a bug that looks like a server bug.
    assert!(
        serde_json::from_str::<CreateRequest>(r#"{"key":"WR","name":"W","nmae":"x"}"#).is_err()
    );
    assert!(serde_json::from_str::<PatchRequest>(r#"{"titel":"x"}"#).is_err());
}
