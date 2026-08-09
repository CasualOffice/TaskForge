//! The rules a file must satisfy before an object key is minted (`docs/28`
//! §Validation, §Limits).
//!
//! # The failure this prevents
//!
//! An object key is a filesystem path on the single-node profile and an S3 key
//! elsewhere. A filename that reaches either unsanitised is a path traversal:
//! `../../etc/passwd` written into `{workspace}/{task}/{id}` escapes the tenant
//! prefix that `docs/32` relies on to keep one tenant's objects unreachable from
//! another's scope.
//!
//! [`object_key`] is therefore built **only** from three UUIDs and never from
//! anything the client sent. The filename is stored as a column, used for
//! `Content-Disposition`, and never used to address storage.

use uuid::Uuid;

/// `docs/28` §Limits: 100 MB default.
pub const DEFAULT_MAX_BYTES: i64 = 100 * 1024 * 1024;
/// `docs/28` §Limits: 2 GB ceiling, workspace-configurable up to it.
pub const ABSOLUTE_MAX_BYTES: i64 = 2 * 1024 * 1024 * 1024;
/// `docs/28` §Limits: 100 files per task.
pub const MAX_FILES_PER_TASK: i64 = 100;
/// `docs/28` §Limits: pre-signed upload TTL.
pub const UPLOAD_TTL_SECONDS: i64 = 15 * 60;
/// `docs/28` §Limits: download URL TTL.
pub const DOWNLOAD_TTL_SECONDS: i64 = 5 * 60;
/// The longest filename accepted. Every input bounded (AGENTS.md).
pub const MAX_FILENAME: usize = 255;
/// A SHA-256 in lowercase hex.
pub const CHECKSUM_LEN: usize = 64;

/// Why a proposed upload was refused.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Refusal {
    EmptyFilename,
    FilenameTooLong,
    /// The filename carries a path separator, a traversal segment, or a NUL.
    FilenameNotSafe,
    ZeroBytes,
    TooLarge {
        limit: i64,
    },
    /// Not 64 lowercase hex characters.
    ChecksumMalformed,
}

/// The object key for an attachment.
///
/// `docs/28`: `{workspace_id}/{task_id}/{attachment_id}`, and `docs/32` makes
/// that prefix the reason "pre-signed URLs are minted only for a key matching
/// the caller's scope". Built from UUIDs alone — there is no parameter here a
/// client can influence, which is what makes traversal impossible rather than
/// filtered.
#[must_use]
pub fn object_key(workspace: Uuid, task: Uuid, attachment: Uuid) -> String {
    format!("{workspace}/{task}/{attachment}")
}

/// The workspace prefix, for the bulk deletes `docs/28` §Orphan and lifecycle
/// cleanup requires on workspace hard-delete.
#[must_use]
pub fn workspace_prefix(workspace: Uuid) -> String {
    format!("{workspace}/")
}

/// Check a proposed upload before anything is written.
///
/// # Errors
///
/// The first [`Refusal`] that applies.
pub fn check(
    filename: &str,
    byte_size: i64,
    checksum: &str,
    max_bytes: i64,
) -> Result<(), Refusal> {
    let name = filename.trim();
    if name.is_empty() {
        return Err(Refusal::EmptyFilename);
    }
    if name.chars().count() > MAX_FILENAME {
        return Err(Refusal::FilenameTooLong);
    }
    if !filename_is_safe(name) {
        return Err(Refusal::FilenameNotSafe);
    }
    if byte_size <= 0 {
        return Err(Refusal::ZeroBytes);
    }
    if byte_size > max_bytes {
        return Err(Refusal::TooLarge { limit: max_bytes });
    }
    if !checksum_is_sha256(checksum) {
        return Err(Refusal::ChecksumMalformed);
    }
    Ok(())
}

/// Whether a filename is safe to store as a column and echo in a header.
///
/// It is never used to address storage — [`object_key`] sees only UUIDs — so
/// this is defence in depth for the two places it *is* used: a
/// `Content-Disposition` header, and any operator who later writes it to a
/// disk.
fn filename_is_safe(name: &str) -> bool {
    !name.contains('/')
        && !name.contains('\\')
        && !name.contains('\0')
        && name != "."
        && name != ".."
        // A leading dot is allowed (`.gitignore`); a traversal segment is not.
        && !name.split(['/', '\\']).any(|part| part == "..")
        // Control characters would let a filename inject a header line break.
        && !name.chars().any(|c| c.is_control())
}

/// Whether a checksum is a lowercase hex SHA-256.
fn checksum_is_sha256(checksum: &str) -> bool {
    checksum.len() == CHECKSUM_LEN
        && checksum
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

/// The effective size limit for a workspace.
///
/// `docs/28`: 100 MB default, "workspace-configurable to 2 GB". A configured
/// value above the ceiling is clamped rather than refused — the ceiling is a
/// property of the system, and an operator who set 5 GB gets 2 GB rather than a
/// workspace whose uploads all fail.
#[must_use]
pub fn size_limit(configured: Option<i64>) -> i64 {
    configured
        .filter(|bytes| *bytes > 0)
        .unwrap_or(DEFAULT_MAX_BYTES)
        .min(ABSOLUTE_MAX_BYTES)
}

#[cfg(test)]
mod tests {
    use super::*;

    const SHA: &str = "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855";

    #[test]
    fn an_object_key_is_built_from_ids_and_nothing_else() {
        // docs/28 and docs/32: the tenant prefix is what makes a key from one
        // workspace unusable in another. A filename cannot reach it.
        let (w, t, a) = (Uuid::now_v7(), Uuid::now_v7(), Uuid::now_v7());
        assert_eq!(object_key(w, t, a), format!("{w}/{t}/{a}"));
        assert!(workspace_prefix(w).starts_with(&w.to_string()));
    }

    #[test]
    fn a_traversing_filename_is_refused() {
        // Defence in depth: it cannot reach the key, and it is still refused.
        for name in [
            "../../etc/passwd",
            "..",
            ".",
            "a/b.png",
            "a\\b.png",
            "with\0nul",
            "line\nbreak.png",
        ] {
            assert!(
                check(name, 10, SHA, DEFAULT_MAX_BYTES).is_err(),
                "accepted {name:?}"
            );
        }
        // A leading dot is an ordinary filename, not a traversal.
        assert!(check(".gitignore", 10, SHA, DEFAULT_MAX_BYTES).is_ok());
    }

    #[test]
    fn sizes_are_bounded_at_both_ends() {
        assert_eq!(
            check("a.png", 0, SHA, DEFAULT_MAX_BYTES).err(),
            Some(Refusal::ZeroBytes)
        );
        assert_eq!(
            check("a.png", DEFAULT_MAX_BYTES + 1, SHA, DEFAULT_MAX_BYTES).err(),
            Some(Refusal::TooLarge {
                limit: DEFAULT_MAX_BYTES
            })
        );
        assert!(check("a.png", DEFAULT_MAX_BYTES, SHA, DEFAULT_MAX_BYTES).is_ok());
    }

    #[test]
    fn a_checksum_must_be_a_lowercase_hex_sha256() {
        // The commit step compares this against the stored object. A malformed
        // one would make that comparison meaningless.
        assert!(check("a.png", 10, &SHA.to_uppercase(), DEFAULT_MAX_BYTES).is_err());
        assert!(check("a.png", 10, "abc", DEFAULT_MAX_BYTES).is_err());
        assert!(check("a.png", 10, &"z".repeat(64), DEFAULT_MAX_BYTES).is_err());
        assert!(check("a.png", 10, SHA, DEFAULT_MAX_BYTES).is_ok());
    }

    #[test]
    fn the_workspace_limit_is_clamped_to_the_systems_ceiling() {
        // docs/28: 100 MB default, 2 GB max. An operator who configures more
        // gets the ceiling, not a workspace where every upload fails.
        assert_eq!(size_limit(None), DEFAULT_MAX_BYTES);
        assert_eq!(size_limit(Some(0)), DEFAULT_MAX_BYTES);
        assert_eq!(size_limit(Some(-1)), DEFAULT_MAX_BYTES);
        assert_eq!(size_limit(Some(5 * 1024 * 1024 * 1024)), ABSOLUTE_MAX_BYTES);
        assert_eq!(size_limit(Some(1024)), 1024);
    }
}
