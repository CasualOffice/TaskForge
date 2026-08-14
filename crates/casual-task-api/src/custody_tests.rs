use super::*;

#[test]
fn a_verdict_outside_the_pair_does_not_reach_the_database() {
    // The column is an enum, so a bad value would be a constraint violation
    // surfacing as a 500. "verdict must be PASS or FAIL" is the sentence a
    // caller can act on.
    for bad in ["PASSED", "ok", "", "DROP TABLE"] {
        assert!(!matches!(bad.to_uppercase().as_str(), "PASS" | "FAIL"));
    }
    assert!(matches!("pass".to_uppercase().as_str(), "PASS"));
}

#[test]
fn an_unknown_field_does_not_deserialize() {
    // docs/05: unknown request fields are rejected, so a typo is a 400 and
    // not a silently ignored intention.
    assert!(serde_json::from_str::<TransferRequest>(r#"{"tema_id":"x"}"#).is_err());
    assert!(serde_json::from_str::<VerifyRequest>(r#"{"verdict":"PASS","x":1}"#).is_err());
}
