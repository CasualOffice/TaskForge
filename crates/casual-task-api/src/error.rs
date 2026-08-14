//! The error envelope (`docs/05` §Errors, `docs/20` error-code registry).
//!
//! ```json
//! { "error": { "code": "TF-WFL-0004", "message": "...", "details": {...},
//!              "request_id": "018f2c...", "docs": "https://..." } }
//! ```
//!
//! # Two properties this type exists to keep
//!
//! **A `request_id` is always present.** `docs/05`: "a `request_id` the user can
//! quote to support". It is a required field of the constructor rather than an
//! `Option`, so an error cannot be built without one.
//!
//! **404 and 403 are not disambiguated.** `docs/04`: an absent resource and an
//! invisible one return the same thing. That is a decision the *caller* can
//! still get wrong by choosing [`ApiError::forbidden`] for a resource the actor
//! cannot see, so the two constructors say which is which in their own
//! documentation.

use axum::Json;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use serde::Serialize;

/// A stable error code from `docs/20`.
///
/// `&'static str` and not a `String`: a code is a compile-time constant from
/// the registry, and one built at runtime is one nobody documented.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct Code(&'static str);

impl Code {
    /// Declare a code. Only this module's constants call it.
    const fn new(code: &'static str) -> Self {
        Self(code)
    }

    /// Adopt a code the model layer already produced.
    ///
    /// `casual-task-search` reports refusals as
    /// [`casual_task_model::ErrorCode`], which is the same registry
    /// (`docs/20`) behind a different newtype — the model crate cannot depend
    /// on this one. Both wrap a `&'static str` from the registry, so carrying
    /// it across is the identity, and doing it here keeps the API crate from
    /// re-deciding what a filter error is called.
    #[must_use]
    pub fn from_registry(code: casual_task_model::ErrorCode) -> Self {
        Self(code.as_str())
    }

    #[must_use]
    pub const fn as_str(&self) -> &'static str {
        self.0
    }
}

/// The codes this crate can currently produce.
///
/// `docs/20` is the registry; these are the subset the server emits before any
/// domain endpoint exists. A code used here that is not in that document is a
/// code no client can look up.
pub mod codes {
    use super::Code;

    /// No credential, or one that is not valid.
    pub const UNAUTHENTICATED: Code = Code::new("TF-AUT-0001");
    /// The credential is valid and is not the kind this endpoint accepts.
    ///
    /// A bearer token is "scoped to one workspace" (`docs/40`), so using one on
    /// a route that is *about* choosing a workspace is outside the contract it
    /// was issued under. Not `TF-AZN-0001`: the fix is a different credential,
    /// not a different role.
    pub const WRONG_CREDENTIAL_TYPE: Code = Code::new("TF-AUT-0013");
    /// The CSRF token was missing or did not verify.
    pub const CSRF: Code = Code::new("TF-AUT-0008");
    /// Absent or invisible — never disambiguated (`docs/04`).
    ///
    /// The generic form of `TF-PRJ-0001` and `TF-TSK-0001`, for the resources
    /// that have no code of their own.
    pub const NOT_FOUND: Code = Code::new("TF-AZN-0008");
    /// The last grant carrying `workspace.owner` cannot be removed or
    /// downgraded (`docs/04` control 4, migration 0021).
    pub const LAST_OWNER: Code = Code::new("TF-AZN-0005");
    /// Control 1 — the role carries a permission the actor does not hold.
    pub const GRANT_CEILING: Code = Code::new("TF-AZN-0003");
    /// Controls 2 and 3 — the assignment is above the actor's scope ceiling, or
    /// it would author a role below workspace scope.
    pub const SCOPE_CEILING: Code = Code::new("TF-AZN-0004");
    /// Control 5 — the actor would add to their own effective set.
    pub const SELF_ELEVATION: Code = Code::new("TF-AZN-0006");
    /// The role name is taken in this workspace.
    pub const ROLE_NAME_TAKEN: Code = Code::new("TF-PRJ-0014");

    /// `UNIQUE (project_id, name)` on `release`. Two things called 2.4.0 in one
    /// project is not a release train, it is a question nobody can answer.
    pub const RELEASE_NAME_TAKEN: Code = Code::new("TF-PRJ-0015");

    /// A release id that names nothing, or whose project the caller cannot
    /// open. The same answer for both, so it cannot be used to probe.
    pub const RELEASE_NOT_FOUND: Code = Code::new("TF-PRJ-0016");

    /// Too many requests. Always carries `Retry-After`.
    pub const RATE_LIMITED: Code = Code::new("TF-LIM-0001");
    /// One bulk request named more tasks than `docs/21` allows.
    pub const BULK_TOO_LARGE: Code = Code::new("TF-LIM-0003");
    /// The service is temporarily unable to answer. Always carries
    /// `Retry-After`.
    pub const UNAVAILABLE: Code = Code::new("TF-SYS-0002");

    /// Designed, not built here. Distinct from `TF-VAL-*`, which means the
    /// caller asked for something that will never exist: this one means the
    /// request is well formed and the capability is scheduled.
    pub const NOT_BUILT: Code = Code::new("TF-SYS-0007");
    /// Anything unhandled.
    pub const INTERNAL: Code = Code::new("TF-SYS-0001");

    // ---------------------------------------------------------------------
    // C-006 / C-008. Every code below is copied from `docs/20`, area and
    // number, rather than invented — a code that is not in that registry is a
    // code no client can look up and the `docs` URL in the envelope 404s.
    // ---------------------------------------------------------------------

    // MFA (C-001, `docs/40` §MFA). The first two were already in `docs/20`
    // waiting for an implementation; the third was added to the registry with
    // this change, which `docs/20` §Rules describes as the way to add one.
    /// This workspace requires MFA and the session has not satisfied it.
    ///
    /// **401, not 403** — and that is the registry's assignment, not a choice
    /// made here. It is a statement about the *credential* being incomplete
    /// rather than about the actor lacking authority, the same distinction
    /// `TF-AUT-0013` draws. A 403 tells a client to give up; a 401 tells it to
    /// strengthen the credential, which is what a step-up is.
    pub const MFA_REQUIRED: Code = Code::new("TF-AUT-0005");
    /// A TOTP or recovery code that did not verify — **for any reason**.
    ///
    /// Wrong, expired, already-replayed, and no-factor-at-all all produce this.
    /// Distinguishing them would tell an attacker holding an observed code that
    /// they had the right one and were merely late.
    pub const MFA_CODE_INVALID: Code = Code::new("TF-AUT-0006");
    /// Enrolment was begun for an account that already has a confirmed factor.
    pub const MFA_ALREADY_ENROLLED: Code = Code::new("TF-AUT-0014");

    /// Malformed request body.
    pub const MALFORMED_BODY: Code = Code::new("TF-VAL-0001");
    /// Unknown field in request. `docs/05`: "silently ignoring a typo'd field
    /// is how clients ship bugs that look like server bugs".
    pub const UNKNOWN_FIELD: Code = Code::new("TF-VAL-0002");
    /// Required field missing.
    pub const MISSING_FIELD: Code = Code::new("TF-VAL-0003");
    /// Field value out of range.
    pub const OUT_OF_RANGE: Code = Code::new("TF-VAL-0004");
    /// Invalid enum value.
    pub const INVALID_ENUM: Code = Code::new("TF-VAL-0005");
    /// Referenced entity not found.
    pub const REFERENCE_NOT_FOUND: Code = Code::new("TF-VAL-0007");

    /// Permission denied — no grant carried it.
    pub const NO_GRANT: Code = Code::new("TF-AZN-0001");
    /// Permission denied — a grant carried it, but not for this object.
    pub const CONSTRAINT_UNSATISFIED: Code = Code::new("TF-AZN-0002");

    /// Invalid or expired cursor.
    pub const BAD_CURSOR: Code = Code::new("TF-QRY-0006");
    /// Page size over limit.
    pub const PAGE_TOO_LARGE: Code = Code::new("TF-QRY-0007");
    /// Unknown filter field.
    pub const UNKNOWN_FILTER_FIELD: Code = Code::new("TF-QRY-0001");
    /// Unknown or unsortable sort field.
    pub const UNSORTABLE_FIELD: Code = Code::new("TF-QRY-0002");
    /// Operator not valid for this field type.
    pub const BAD_OPERATOR: Code = Code::new("TF-QRY-0003");
    /// Too many filter clauses.
    pub const TOO_MANY_CLAUSES: Code = Code::new("TF-QRY-0004");
    /// Filter nesting too deep.
    pub const FILTER_TOO_DEEP: Code = Code::new("TF-QRY-0005");
    /// Search query too long.
    pub const SEARCH_TOO_LONG: Code = Code::new("TF-QRY-0008");
    /// A symbol (`@me`, `+7d`) this server does not know.
    ///
    /// `docs/20` has no code for it, so it reports as the operator/value code:
    /// an unrecognised symbol is a malformed value for the field it was written
    /// on. Recorded in `docs/14` as a registry gap rather than a new area.
    pub const UNKNOWN_SYMBOL: Code = Code::new("TF-QRY-0003");

    /// Project not found or not visible — never disambiguated.
    pub const PROJECT_NOT_FOUND: Code = Code::new("TF-PRJ-0001");
    /// Project key already in use.
    pub const PROJECT_KEY_TAKEN: Code = Code::new("TF-PRJ-0002");
    /// Project key is immutable (ADR-007).
    pub const PROJECT_KEY_IMMUTABLE: Code = Code::new("TF-PRJ-0003");
    /// Project key format invalid.
    pub const PROJECT_KEY_FORMAT: Code = Code::new("TF-PRJ-0004");

    /// Task not found or not visible — never disambiguated.
    pub const TASK_NOT_FOUND: Code = Code::new("TF-TSK-0001");
    /// Assignee is not a member of the project.
    pub const ASSIGNEE_NOT_PROJECT_MEMBER: Code = Code::new("TF-TSK-0005");
    /// A dependency that would close a loop (ADR-019, `docs/20` TF-TSK-0003).
    pub const DEPENDENCY_CYCLE: Code = Code::new("TF-TSK-0003");
    /// Parent task must be in the same project (ADR-018).
    pub const PARENT_OUT_OF_PROJECT: Code = Code::new("TF-TSK-0006");

    /// Status cannot be set directly — use a transition (`docs/23`).
    pub const STATUS_NOT_DIRECTLY_WRITABLE: Code = Code::new("TF-WFL-0001");
    /// No such transition in this workflow.
    pub const NO_SUCH_TRANSITION: Code = Code::new("TF-WFL-0002");
    /// The transition requires a permission the actor lacks.
    pub const TRANSITION_PERMISSION: Code = Code::new("TF-WFL-0003");
    /// Required fields missing for the target status.
    pub const TRANSITION_FIELDS_MISSING: Code = Code::new("TF-WFL-0004");
    /// An environment holding tasks was deleted without a migration target.
    pub const ENVIRONMENT_IN_USE: Code = Code::new("TF-PRJ-0005");
    /// The environment name is taken inside this project.
    pub const ENVIRONMENT_NAME_TAKEN: Code = Code::new("TF-PRJ-0009");
    /// A status holding tasks was deleted without `migrate_to` (`docs/23`).
    pub const STATUS_HOLDS_TASKS: Code = Code::new("TF-WFL-0006");
    /// A workflow must have exactly one initial status.
    pub const INITIAL_STATUS_RULE: Code = Code::new("TF-WFL-0007");
    /// The status named belongs to a different workflow.
    pub const STATUS_WRONG_WORKFLOW: Code = Code::new("TF-WFL-0008");
    /// The status name is taken inside this workflow.
    pub const STATUS_NAME_TAKEN: Code = Code::new("TF-WFL-0009");
    /// That transition already exists between those two statuses.
    pub const TRANSITION_EXISTS: Code = Code::new("TF-WFL-0010");
    /// More tasks would move than a request may carry — `docs/23` puts the
    /// ceiling at 10,000 and runs the rest as a tracked job.
    pub const MIGRATION_TOO_LARGE: Code = Code::new("TF-WFL-0011");

    /// Blocking dependencies unresolved.
    pub const BLOCKED_BY_DEPENDENCIES: Code = Code::new("TF-WFL-0005");

    /// Version conflict.
    pub const VERSION_CONFLICT: Code = Code::new("TF-CNC-0001");
    /// The export exists and is not finished. Distinct from `404`: "not yours"
    /// and "not yet" are different facts, and only one of them is worth
    /// retrying (docs/38 §Export is a job, not a request).
    ///
    /// `CNC` and not a new `EXP` area: docs/20 declares a closed set of areas,
    /// and `the_area_of_every_code_is_one_the_registry_declares` refuses a code
    /// outside it. A not-yet-ready artefact is a state conflict, which is what
    /// this area already means.
    pub const EXPORT_NOT_READY: Code = Code::new("TF-CNC-0004");
    /// `If-Match` required.
    pub const IF_MATCH_REQUIRED: Code = Code::new("TF-CNC-0002");
    /// Malformed `If-Match`.
    pub const IF_MATCH_MALFORMED: Code = Code::new("TF-CNC-0003");

    /// A request with this idempotency key is already in progress.
    pub const IDEMPOTENCY_IN_PROGRESS: Code = Code::new("TF-IDM-0001");
    /// Idempotency key reused with a different body.
    pub const IDEMPOTENCY_BODY_CHANGED: Code = Code::new("TF-IDM-0002");
    /// Idempotency key required.
    pub const IDEMPOTENCY_REQUIRED: Code = Code::new("TF-IDM-0003");

    /// File exceeds the size limit.
    pub const ATTACHMENT_TOO_LARGE: Code = Code::new("TF-ATT-0001");
    /// The bytes are a content type this system will not store — markup.
    pub const ATTACHMENT_TYPE_REFUSED: Code = Code::new("TF-ATT-0002");
    /// The declared type does not match what the bytes are.
    pub const ATTACHMENT_TYPE_MISMATCH: Code = Code::new("TF-ATT-0003");
    /// Upload not found or expired.
    pub const ATTACHMENT_NOT_FOUND: Code = Code::new("TF-ATT-0005");
    /// Malware detected.
    pub const ATTACHMENT_INFECTED: Code = Code::new("TF-ATT-0006");
    /// Scan pending — not yet available.
    pub const ATTACHMENT_SCAN_PENDING: Code = Code::new("TF-ATT-0007");
    /// The uploaded object is not the size that was declared.
    pub const ATTACHMENT_SIZE_MISMATCH: Code = Code::new("TF-ATT-0009");
    /// The scan did not complete and the file will not be served.
    pub const ATTACHMENT_SCAN_FAILED: Code = Code::new("TF-ATT-0010");
    /// This task already holds the maximum number of attachments.
    pub const ATTACHMENT_TOO_MANY: Code = Code::new("TF-ATT-0011");

    /// A workspace would lose its last member.
    pub const LAST_MEMBER: Code = Code::new("TF-PRJ-0006");
    /// The slug is taken by another workspace.
    pub const SLUG_TAKEN: Code = Code::new("TF-PRJ-0007");
    /// The team name is taken inside this workspace.
    pub const TEAM_NAME_TAKEN: Code = Code::new("TF-PRJ-0008");
    /// The milestone name is taken inside this project.
    pub const MILESTONE_NAME_TAKEN: Code = Code::new("TF-PRJ-0010");
    /// The tag name is taken at that scope — workspace, or that one project.
    pub const TAG_NAME_TAKEN: Code = Code::new("TF-PRJ-0011");
    /// A project already holds the maximum number of milestones.
    pub const MILESTONE_LIMIT: Code = Code::new("TF-PRJ-0012");
    /// A workspace already holds the maximum number of tags.
    pub const TAG_LIMIT: Code = Code::new("TF-PRJ-0013");

    /// Every code this binary can emit.
    ///
    /// The registry gate walks this list, so a code missing from it is a
    /// code whose `docs` URL is never checked against `docs/20`.
    pub const ALL: &[Code] = &[
        UNAUTHENTICATED,
        WRONG_CREDENTIAL_TYPE,
        CSRF,
        NOT_FOUND,
        LAST_OWNER,
        GRANT_CEILING,
        SCOPE_CEILING,
        SELF_ELEVATION,
        ROLE_NAME_TAKEN,
        RATE_LIMITED,
        BULK_TOO_LARGE,
        UNAVAILABLE,
        INTERNAL,
        MALFORMED_BODY,
        UNKNOWN_FIELD,
        MISSING_FIELD,
        OUT_OF_RANGE,
        INVALID_ENUM,
        REFERENCE_NOT_FOUND,
        NO_GRANT,
        CONSTRAINT_UNSATISFIED,
        BAD_CURSOR,
        PAGE_TOO_LARGE,
        UNKNOWN_FILTER_FIELD,
        UNSORTABLE_FIELD,
        BAD_OPERATOR,
        TOO_MANY_CLAUSES,
        FILTER_TOO_DEEP,
        SEARCH_TOO_LONG,
        PROJECT_NOT_FOUND,
        PROJECT_KEY_TAKEN,
        PROJECT_KEY_IMMUTABLE,
        PROJECT_KEY_FORMAT,
        TASK_NOT_FOUND,
        DEPENDENCY_CYCLE,
        ASSIGNEE_NOT_PROJECT_MEMBER,
        PARENT_OUT_OF_PROJECT,
        STATUS_NOT_DIRECTLY_WRITABLE,
        NO_SUCH_TRANSITION,
        TRANSITION_PERMISSION,
        TRANSITION_FIELDS_MISSING,
        ENVIRONMENT_IN_USE,
        ENVIRONMENT_NAME_TAKEN,
        STATUS_HOLDS_TASKS,
        INITIAL_STATUS_RULE,
        STATUS_WRONG_WORKFLOW,
        STATUS_NAME_TAKEN,
        TRANSITION_EXISTS,
        MIGRATION_TOO_LARGE,
        BLOCKED_BY_DEPENDENCIES,
        VERSION_CONFLICT,
        EXPORT_NOT_READY,
        IF_MATCH_REQUIRED,
        IF_MATCH_MALFORMED,
        IDEMPOTENCY_IN_PROGRESS,
        IDEMPOTENCY_BODY_CHANGED,
        IDEMPOTENCY_REQUIRED,
        ATTACHMENT_TOO_LARGE,
        ATTACHMENT_TYPE_REFUSED,
        ATTACHMENT_TYPE_MISMATCH,
        ATTACHMENT_NOT_FOUND,
        ATTACHMENT_INFECTED,
        ATTACHMENT_SCAN_PENDING,
        ATTACHMENT_SIZE_MISMATCH,
        ATTACHMENT_SCAN_FAILED,
        ATTACHMENT_TOO_MANY,
        LAST_MEMBER,
        SLUG_TAKEN,
        TEAM_NAME_TAKEN,
        MILESTONE_NAME_TAKEN,
        TAG_NAME_TAKEN,
        MILESTONE_LIMIT,
        TAG_LIMIT,
    ];
}

include!("api_error.rs");
#[cfg(test)]
#[path = "error_tests.rs"]
mod tests;
