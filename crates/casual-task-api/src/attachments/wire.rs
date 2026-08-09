//! The attachment request and response shapes (`docs/05` §Conventions).
//!
//! # What is deliberately absent from the response
//!
//! `object_key` and `scan_detail` never leave the server. The key is the
//! storage address and `docs/32` makes its unguessability part of tenant
//! isolation; the scan detail names detection signatures, which is
//! reconnaissance. A client that needs to address an attachment uses its id,
//! and a client that needs to fetch it asks for a signed URL.

use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// What a client sees. `snake_case`, RFC 3339 UTC (`docs/05`).
#[derive(Debug, Serialize)]
pub struct AttachmentView {
    pub id: Uuid,
    pub task_id: Uuid,
    pub filename: String,
    /// The type sniffed from magic bytes, never the client's declaration
    /// (`docs/28`).
    pub content_type: String,
    pub byte_size: i64,
    pub checksum: String,
    /// `PENDING` | `CLEAN` | `INFECTED` | `FAILED`. Present so a client can
    /// show "scanning…" rather than a broken download link.
    pub scan_status: String,
    pub uploaded_by: Uuid,
    pub created_at: String,
}

/// `POST /api/v1/tasks/{id}/attachments`.
///
/// `content_type` is accepted and used **only** to pin the pre-signed policy
/// (`docs/28` §Validation). It is not stored, and the commit step overrides it
/// from the bytes. It is a required field rather than an optional one so a
/// client cannot silently skip the declaration that its upload will be checked
/// against.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PresignRequest {
    pub filename: String,
    pub content_type: String,
    pub byte_size: i64,
    /// Lowercase hex SHA-256 of the bytes the client is about to upload.
    pub checksum: String,
}

/// The pre-sign response: everything the client needs to upload, and nothing
/// about where the object really lives.
#[derive(Debug, Serialize)]
pub struct PresignResponse {
    pub attachment_id: Uuid,
    pub upload_url: String,
    /// `docs/28` step 1 returns headers the client must send. The signature
    /// pins the method and the key; the content type is pinned here.
    pub headers: Vec<(String, String)>,
    pub expires_in: i64,
}

/// The commit response. `202`, because the scan has not finished
/// (`docs/28` step 3).
#[derive(Debug, Serialize)]
pub struct CommitResponse {
    pub attachment_id: Uuid,
    pub scan_status: String,
    /// The verified type, so a client can render the right icon while it waits.
    pub content_type: String,
}
