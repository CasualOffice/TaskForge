use super::*;

#[test]
fn the_member_types_are_the_ones_the_check_constraint_allows() {
    // Migration 0002: CHECK (member_type IN ('MEMBER','GUEST')). A value
    // accepted here and refused there would abort a transaction that has
    // already written its audit row.
    let migration = include_str!("../../../migrations/0002_tenancy_and_identity.sql");
    for member_type in MEMBER_TYPES {
        assert!(
            migration.contains(&format!("'{member_type}'")),
            "{member_type} is offered by the API and refused by the schema"
        );
    }
    assert_eq!(MEMBER_TYPES.len(), 2);
}
