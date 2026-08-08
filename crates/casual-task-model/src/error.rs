//! Typed errors carrying registry codes. See `docs/20-ERROR-CODE-REGISTRY.md`.
//!
//! Every error surfaced to a client carries a stable `TF-AREA-NNNN` code so
//! clients can react programmatically, logs are greppable, and support can be
//! answered from a code rather than a screenshot.
//!
//! Codes are append-only: never reused, never repurposed.

use std::fmt;

/// A stable, namespaced diagnostic code.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ErrorCode(&'static str);

impl ErrorCode {
    pub const fn new(code: &'static str) -> Self {
        Self(code)
    }

    pub fn as_str(&self) -> &'static str {
        self.0
    }
}

impl fmt::Display for ErrorCode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.0)
    }
}

/// The subset of the registry the model layer itself can raise. Each area's
/// crate declares its own; the `error-registry` CI gate checks that every code
/// emitted anywhere exists in `docs/20-ERROR-CODE-REGISTRY.md`.
pub mod codes {
    use super::ErrorCode;

    // Authorization — docs/04
    pub const AZN_NO_GRANT: ErrorCode = ErrorCode::new("TF-AZN-0001");
    pub const AZN_CONSTRAINT_UNSATISFIED: ErrorCode = ErrorCode::new("TF-AZN-0002");
    pub const AZN_GRANT_CEILING: ErrorCode = ErrorCode::new("TF-AZN-0003");
    pub const AZN_SCOPE_CEILING: ErrorCode = ErrorCode::new("TF-AZN-0004");
    pub const AZN_LAST_OWNER: ErrorCode = ErrorCode::new("TF-AZN-0005");
    pub const AZN_SELF_ELEVATION: ErrorCode = ErrorCode::new("TF-AZN-0006");

    // Query — docs/26, docs/27
    pub const QRY_UNKNOWN_FIELD: ErrorCode = ErrorCode::new("TF-QRY-0001");
    pub const QRY_UNSORTABLE_FIELD: ErrorCode = ErrorCode::new("TF-QRY-0002");
    pub const QRY_BAD_OPERATOR: ErrorCode = ErrorCode::new("TF-QRY-0003");
    pub const QRY_TOO_MANY_CLAUSES: ErrorCode = ErrorCode::new("TF-QRY-0004");
    pub const QRY_TOO_DEEP: ErrorCode = ErrorCode::new("TF-QRY-0005");
    pub const QRY_BAD_CURSOR: ErrorCode = ErrorCode::new("TF-QRY-0006");

    // Concurrency — docs/24
    pub const CNC_VERSION_CONFLICT: ErrorCode = ErrorCode::new("TF-CNC-0001");
    pub const CNC_IF_MATCH_REQUIRED: ErrorCode = ErrorCode::new("TF-CNC-0002");

    // Workflow — docs/23
    pub const WFL_DIRECT_STATUS_WRITE: ErrorCode = ErrorCode::new("TF-WFL-0001");
    pub const WFL_NO_SUCH_TRANSITION: ErrorCode = ErrorCode::new("TF-WFL-0002");
    pub const WFL_BLOCKED_BY_DEPENDENCY: ErrorCode = ErrorCode::new("TF-WFL-0005");

    // Task — docs/03
    pub const TSK_NOT_FOUND: ErrorCode = ErrorCode::new("TF-TSK-0001");
    pub const TSK_DEPENDENCY_CYCLE: ErrorCode = ErrorCode::new("TF-TSK-0003");
    pub const TSK_NESTING_LIMIT: ErrorCode = ErrorCode::new("TF-TSK-0004");
}

/// The error shape every layer converts into.
///
/// `details` returns **every** violation, not the first — a form that reveals
/// its requirements one round-trip at a time is a bad form (`docs/05-API-SPEC.md`).
#[derive(Debug, Clone)]
pub struct Error {
    pub code: ErrorCode,
    pub message: String,
    pub details: Vec<(String, String)>,
}

impl Error {
    pub fn new(code: ErrorCode, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
            details: Vec::new(),
        }
    }

    pub fn with_detail(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.details.push((key.into(), value.into()));
        self
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "[{}] {}", self.code, self.message)
    }
}

impl std::error::Error for Error {}

pub type Result<T> = std::result::Result<T, Error>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn error_displays_with_its_code() {
        let e = Error::new(codes::TSK_NOT_FOUND, "Task not found or not visible");
        assert_eq!(e.to_string(), "[TF-TSK-0001] Task not found or not visible");
    }

    #[test]
    fn details_accumulate_so_all_violations_report_at_once() {
        let e = Error::new(codes::QRY_UNKNOWN_FIELD, "bad filter")
            .with_detail("field", "asignee")
            .with_detail("field", "duedate");
        assert_eq!(e.details.len(), 2);
    }
}
