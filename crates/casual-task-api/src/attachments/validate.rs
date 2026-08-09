//! Field rules for the attachment endpoints.
//!
//! The substance lives in `casual-task-attachment::policy`, which has no HTTP
//! and no database and is therefore testable on its own. This module's only job
//! is turning a [`Refusal`] into the documented status and code — so the rule
//! and its error code cannot drift, and a second endpoint cannot enforce the
//! same rule slightly differently.

use casual_task_app::attachment::{Refusal, policy};

use crate::error::{ApiError, codes};

/// Check a pre-sign request.
///
/// # Errors
///
/// `400` for a malformed field, `422` for a size over the workspace's limit —
/// `docs/05` splits them that way: a filename with a slash is malformed, and a
/// file that is simply too big is a domain rule.
pub fn presign(
    filename: &str,
    byte_size: i64,
    checksum: &str,
    max_bytes: i64,
    request_id: &str,
) -> Result<(), ApiError> {
    policy::check(filename, byte_size, checksum, max_bytes).map_err(|refusal| match refusal {
        // 413, which is what `docs/20` assigns TF-ATT-0001 — not 422. The
        // distinction is real: 413 is about the request, and a client can act
        // on it without parsing a body.
        Refusal::TooLarge { limit } => ApiError::new(
            axum::http::StatusCode::PAYLOAD_TOO_LARGE,
            codes::ATTACHMENT_TOO_LARGE,
            "That file is larger than this workspace allows",
            request_id,
        )
        .with_details(serde_json::json!({ "limit_bytes": limit })),
        Refusal::EmptyFilename | Refusal::FilenameTooLong | Refusal::FilenameNotSafe => {
            ApiError::bad_request(
                codes::OUT_OF_RANGE,
                "filename must be 1-255 characters and must not contain a path \
                 separator, a traversal segment, or a control character",
                request_id,
            )
        }
        Refusal::ZeroBytes => ApiError::bad_request(
            codes::OUT_OF_RANGE,
            "byte_size must be greater than zero",
            request_id,
        ),
        Refusal::ChecksumMalformed => ApiError::bad_request(
            codes::OUT_OF_RANGE,
            "checksum must be a lowercase hex SHA-256",
            request_id,
        ),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::StatusCode;

    const SHA: &str = "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855";

    #[test]
    fn a_size_refusal_is_413_and_a_shape_refusal_is_400() {
        // docs/20 assigns TF-ATT-0001 a 413. A filename with a slash is
        // malformed and is a 400.
        let big = presign("a.png", 10_000, SHA, 100, "r").expect_err("too large");
        assert_eq!(big.status(), StatusCode::PAYLOAD_TOO_LARGE);
        assert_eq!(big.code(), codes::ATTACHMENT_TOO_LARGE);

        let bad = presign("../a.png", 10, SHA, 100, "r").expect_err("traversal");
        assert_eq!(bad.status(), StatusCode::BAD_REQUEST);
    }

    #[test]
    fn the_limit_is_named_so_a_client_can_show_it() {
        let error = presign("a.png", 10_000, SHA, 100, "r").expect_err("too large");
        let rendered = format!("{error:?}");
        assert!(rendered.contains("100"), "the limit is not in the details");
    }

    #[test]
    fn a_valid_request_passes() {
        assert!(presign("report.pdf", 1024, SHA, 100_000, "r").is_ok());
    }
}
