//! Field-level validation.
//!
//! Pure functions over request values, so they are testable without a database
//! and cannot accidentally reach one. Every one returns the documented error
//! code rather than a bare `bool` — a validator that says only "no" forces the
//! caller to invent the reason, and callers invent inconsistently.

use time::OffsetDateTime;
use time::format_description::well_known::Rfc3339;

use crate::error::{ApiError, codes};

pub(crate) fn validated_title<'a>(title: &'a str, request_id: &str) -> Result<&'a str, ApiError> {
    let trimmed = title.trim();
    // migrations/0005: CHECK (length(title) BETWEEN 1 AND 512). Checked here so
    // the caller gets a described bound rather than a 500 from a constraint.
    if trimmed.is_empty() || trimmed.chars().count() > 512 {
        return Err(ApiError::bad_request(
            codes::OUT_OF_RANGE,
            "title must be between 1 and 512 characters",
            request_id,
        ));
    }
    Ok(trimmed)
}

pub(crate) fn one_of<'a>(
    value: Option<&'a str>,
    allowed: &[&'static str],
    default: &'a str,
    field: &str,
    request_id: &str,
) -> Result<&'a str, ApiError> {
    let Some(value) = value else {
        return Ok(default);
    };
    if allowed.contains(&value) {
        Ok(value)
    } else {
        Err(ApiError::bad_request(
            codes::INVALID_ENUM,
            format!("{field} is not one of the permitted values"),
            request_id,
        )
        .with_details(serde_json::json!({ "field": field, "allowed": allowed })))
    }
}

/// Parse an optional, nullable RFC 3339 timestamp from a patch.
pub(crate) fn optional_timestamp(
    value: Option<&Option<String>>,
    field: &str,
    request_id: &str,
) -> Result<Option<Option<OffsetDateTime>>, ApiError> {
    match value {
        None => Ok(None),
        Some(None) => Ok(Some(None)),
        Some(Some(raw)) => OffsetDateTime::parse(raw, &Rfc3339)
            .map(|at| Some(Some(at)))
            .map_err(|_| {
                ApiError::bad_request(
                    codes::MALFORMED_BODY,
                    format!("{field} must be an RFC 3339 timestamp"),
                    request_id,
                )
            }),
    }
}

/// Whether a supplied field value counts as absent for step 6.
///
/// `docs/23`: required fields must be "present and non-empty". A `null`, an
/// empty string, or an empty list is a field the user did not fill in, and
/// accepting one would make a required field a formality.
pub(crate) fn is_empty_value(value: &serde_json::Value) -> bool {
    match value {
        serde_json::Value::Null => true,
        serde_json::Value::String(s) => s.trim().is_empty(),
        serde_json::Value::Array(a) => a.is_empty(),
        serde_json::Value::Object(o) => o.is_empty(),
        _ => false,
    }
}

// ---------------------------------------------------------------------------
// Assignees and tags
// ---------------------------------------------------------------------------

/// The stored spelling of a state.
///
/// Exhaustive, so a sixth state cannot appear without deciding what it is
/// called on disk.
pub(crate) const fn state_wire(state: casual_task_model::TaskState) -> &'static str {
    use casual_task_model::TaskState;
    match state {
        TaskState::Backlog => "BACKLOG",
        TaskState::Planned => "PLANNED",
        TaskState::Active => "ACTIVE",
        TaskState::Completed => "COMPLETED",
        TaskState::Canceled => "CANCELED",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tasks::*;
    use casual_task_persistence::task::TaskRow;
    use uuid::Uuid;

    #[test]
    fn the_enums_match_the_ones_the_schema_declares() {
        // A value the API accepts and the enum does not is a 500 from a cast.
        let migration = include_str!("../../../../migrations/0001_extensions_and_types.sql");
        for value in TASK_TYPES.iter().chain(PRIORITIES.iter()) {
            assert!(
                migration.contains(&format!("'{value}'")),
                "{value} is not declared in migration 0001"
            );
        }
        assert_eq!(TASK_TYPES.len(), 5);
        assert_eq!(PRIORITIES.len(), 5);
    }

    #[test]
    fn every_state_has_a_stored_spelling_the_schema_knows() {
        let migration = include_str!("../../../../migrations/0001_extensions_and_types.sql");
        for state in casual_task_model::TaskState::ALL {
            assert!(
                migration.contains(&format!("'{}'", state_wire(state))),
                "{state:?} maps to a value task_state does not declare"
            );
        }
    }

    #[test]
    fn a_title_is_bounded_at_the_schemas_bound() {
        assert!(validated_title("x", "r").is_ok());
        assert!(validated_title(&"x".repeat(512), "r").is_ok());
        assert!(validated_title(&"x".repeat(513), "r").is_err());
        assert!(validated_title("   ", "r").is_err());
        let migration = include_str!("../../../../migrations/0005_tasks.sql");
        assert!(
            migration.contains("length(title) BETWEEN 1 AND 512"),
            "the schema's title bound moved; this check must move with it"
        );
    }

    #[test]
    fn a_create_cannot_name_a_status() {
        // docs/23: status is never written directly, and a create is not an
        // exception. `deny_unknown_fields` is what enforces it.
        assert!(
            serde_json::from_str::<CreateRequest>(
                r#"{"title":"t","status_id":"018f2c9e-0000-7000-8000-000000000001"}"#
            )
            .is_err()
        );
        assert!(serde_json::from_str::<CreateRequest>(r#"{"title":"t","state":"DONE"}"#).is_err());
    }

    #[test]
    fn a_key_reads_as_project_key_and_number() {
        // docs/05's pagination example shows "key": "WR-125". It spans two
        // tables, so it is composed on read (D-051).
        let row = TaskRow {
            id: Uuid::now_v7(),
            workspace_id: Uuid::now_v7(),
            project_id: Uuid::now_v7(),
            number: 125,
            title: "t".into(),
            description: None,
            task_type: "TASK".into(),
            priority: "NONE".into(),
            status_id: Uuid::now_v7(),
            state: "BACKLOG".into(),
            reporter_id: Uuid::now_v7(),
            team_id: None,
            environment_id: None,
            milestone_id: None,
            parent_id: None,
            start_at: None,
            due_at: None,
            position: "11111111".into(),
            created_at: OffsetDateTime::UNIX_EPOCH,
            created_by: Uuid::now_v7(),
            updated_at: OffsetDateTime::UNIX_EPOCH,
            updated_by: None,
            version: 1,
            rank: None,
            archived_at: None,
            is_blocked: false,
        };
        assert_eq!(view(&row, "WR").key, "WR-125");
    }

    #[test]
    fn an_unknown_enum_value_is_refused_rather_than_defaulted() {
        assert!(one_of(None, TASK_TYPES, "TASK", "type", "r").is_ok());
        assert_eq!(
            one_of(Some("EPIC"), TASK_TYPES, "TASK", "type", "r")
                .err()
                .map(|e| e.code()),
            Some(codes::INVALID_ENUM)
        );
    }
}
