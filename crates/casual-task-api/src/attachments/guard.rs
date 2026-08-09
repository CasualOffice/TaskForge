//! Who may reach an attachment, and in what state it may be served.
//!
//! # Why this is its own module
//!
//! An attachment has **two** gates, and they are easy to conflate:
//!
//! 1. *Visibility* — can this actor see the task it hangs off? That is the
//!    task's rule, and it is reused rather than restated: `tasks::guard`
//!    already answers it, and a second copy here would be the second answer
//!    `AGENTS.md` §Module size warns about.
//! 2. *State* — is this file safe to hand over? That is this module's own, and
//!    it is the one `docs/28` exists for. An attachment can be perfectly
//!    visible and still be an unscanned executable.
//!
//! Keeping them together in one function is what makes it impossible to write a
//! download handler that checks the first and forgets the second.

use casual_task_model::permission;
use casual_task_persistence::attachment::{self, AttachmentRow, CLEAN};
use casual_task_persistence::{Scoped, task::TaskRow};
use uuid::Uuid;

use crate::context::Context;
use crate::error::{ApiError, codes};
use crate::tasks::guard as task_guard;

/// The task an attachment belongs to, if the actor may see it **and** holds
/// `permission` on it.
///
/// Both answers come from the task, because an attachment has no authority of
/// its own — `docs/04` resolves permissions per project, and an attachment is
/// in whatever project its task is.
///
/// # Errors
///
/// `404` when the task is not visible, `403` without the permission.
pub async fn task_for(
    scoped: &mut Scoped<'_>,
    ctx: &Context,
    task_id: Uuid,
    wanted: casual_task_model::Permission,
    request_id: &str,
) -> Result<TaskRow, ApiError> {
    let (task, _) = task_guard::visible(scoped, ctx, task_id, request_id).await?;
    task_guard::authorize_on_task(scoped, ctx, &task, wanted, request_id).await?;
    Ok(task)
}

/// An attachment the actor may **download**.
///
/// Three conditions, in the order that keeps them distinguishable:
///
/// 1. The row is visible at all — committed, undeleted, in this tenant. Absent
///    and invisible are one `404` (`docs/04`).
/// 2. The actor may read attachments on its task.
/// 3. The scan says `CLEAN`.
///
/// The third is **not** folded into the first. A `404` for a file that is
/// merely still being scanned would tell the person who just uploaded it that
/// their file vanished; `409` with its own code tells them to wait, which is
/// true and actionable.
///
/// # Errors
///
/// `404` invisible, `403` unpermitted, `409` still scanning, `422` infected.
pub async fn downloadable(
    scoped: &mut Scoped<'_>,
    ctx: &Context,
    id: Uuid,
    request_id: &str,
) -> Result<AttachmentRow, ApiError> {
    let row = attachment::find_visible(scoped, id)
        .await
        .map_err(|error| {
            tracing::error!(%error, "reading the attachment failed");
            ApiError::internal(request_id)
        })?
        .ok_or_else(|| ApiError::missing(codes::ATTACHMENT_NOT_FOUND, request_id))?;

    task_for(
        scoped,
        ctx,
        row.task_id,
        permission::TASK_ATTACHMENT_READ,
        request_id,
    )
    .await?;

    scanned_clean(&row, request_id)?;
    Ok(row)
}

/// Refuse anything the scanner has not cleared.
///
/// `docs/28` step 4 makes `CLEAN` the only verdict that produces a visible
/// file, and D-062 makes "no scanner configured" fail closed — an attachment
/// stays `PENDING` forever and is never downloadable, rather than being served
/// unscanned.
///
/// Written as a `match` over the verdict with no `_` arm, so a fifth scan state
/// cannot be added without deciding whether it may be downloaded.
///
/// # Errors
///
/// `409` while pending, `422` when infected or the scan failed.
pub fn scanned_clean(row: &AttachmentRow, request_id: &str) -> Result<(), ApiError> {
    match row.scan_status.as_str() {
        CLEAN => Ok(()),
        "PENDING" => Err(ApiError::conflict(
            codes::ATTACHMENT_SCAN_PENDING,
            "This file has not finished being scanned",
            request_id,
        )),
        "INFECTED" => Err(ApiError::unprocessable(
            codes::ATTACHMENT_INFECTED,
            "This file was quarantined",
            request_id,
        )),
        "FAILED" => Err(ApiError::unprocessable(
            codes::ATTACHMENT_SCAN_FAILED,
            "This file could not be scanned and will not be served",
            request_id,
        )),
        // A verdict the code does not know is not a reason to serve the file.
        other => {
            tracing::error!(verdict = other, "unknown scan verdict; refusing");
            Err(ApiError::unprocessable(
                codes::ATTACHMENT_SCAN_FAILED,
                "This file could not be scanned and will not be served",
                request_id,
            ))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::StatusCode;
    use time::OffsetDateTime;

    fn row(scan_status: &str) -> AttachmentRow {
        AttachmentRow {
            id: Uuid::now_v7(),
            task_id: Uuid::now_v7(),
            object_key: "w/t/a".into(),
            filename: "a.png".into(),
            content_type: "image/png".into(),
            byte_size: 10,
            checksum: "x".repeat(64),
            scan_status: scan_status.to_owned(),
            committed_at: Some(OffsetDateTime::UNIX_EPOCH),
            uploaded_by: Uuid::now_v7(),
            created_at: OffsetDateTime::UNIX_EPOCH,
        }
    }

    #[test]
    fn only_clean_is_served() {
        assert!(scanned_clean(&row("CLEAN"), "r").is_ok());
        for refused in ["PENDING", "INFECTED", "FAILED", "SOMETHING_NEW"] {
            assert!(
                scanned_clean(&row(refused), "r").is_err(),
                "{refused} was served"
            );
        }
    }

    #[test]
    fn pending_is_a_409_and_not_a_404() {
        // The person who just uploaded it must be told to wait, not that their
        // file does not exist.
        let error = scanned_clean(&row("PENDING"), "r").expect_err("pending");
        assert_eq!(error.status(), StatusCode::CONFLICT);
        assert_eq!(error.code(), codes::ATTACHMENT_SCAN_PENDING);
    }

    #[test]
    fn an_unknown_verdict_fails_closed() {
        // D-062's shape, one level down: the default when the system does not
        // know is "do not serve".
        let error = scanned_clean(&row("WHO_KNOWS"), "r").expect_err("unknown");
        assert_eq!(error.status(), StatusCode::UNPROCESSABLE_ENTITY);
    }
}
