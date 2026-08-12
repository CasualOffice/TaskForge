//! Step 4 of `docs/28`: the scan that makes an attachment visible.
//!
//! # Why an upload does nothing until this consumer runs
//!
//! `commit` writes the row, verifies the bytes and records
//! `attachment.uploaded` — and stops. `committed_at` stays `NULL`, and every
//! read of an attachment requires it to be set, so the file is stored and
//! invisible. That is the whole point: a forgotten `WHERE` clause cannot leak an
//! unscanned file, because there is no state in which one is listed.
//!
//! Until this existed, nothing ever set it. The pipeline stored files that could
//! never be seen — correct by `docs/28`'s own rule, and useless.
//!
//! # Why "no scanner" is not "clean"
//!
//! D-062, countersigned: a deployment with no scanner leaves the row `PENDING`
//! and it is never downloadable. So when no scanner is configured this consumer
//! **acknowledges the delivery and changes nothing**, loudly. The alternative —
//! treating an unreachable scanner as a pass — is the silent lie that decision
//! exists to forbid, and it is not one this code may pick.
//!
//! The cost is stated rather than hidden: files uploaded while no scanner was
//! configured stay invisible after one is added, because the event has already
//! been delivered. Re-scanning them is a job that does not exist yet.
//!
//! # Why a failed scan is not a verdict either
//!
//! An unreachable daemon and a timed-out scan return `Err`, which leaves the
//! delivery unacknowledged and lets the dispatcher retry with backoff. `docs/28`
//! step 4 quarantines after three attempts; that escalation is the dispatcher's
//! existing dead-letter path, not a second mechanism here.

use std::sync::Arc;

use casual_task_infra::{ObjectStore, Scanner, Verdict};
use casual_task_model::{WorkspaceId, WorkspaceScope};
use casual_task_persistence::dispatch::Claimed;
use casual_task_persistence::{Scoped, attachment};
use sqlx::PgPool;

use crate::dispatcher::Consumer;

/// Must match the entry in [`casual_task_persistence::CONSUMERS`], or the loop
/// polls forever and is handed nothing.
pub const NAME: &str = "attachment_scan";

/// Scans committed-but-unscanned attachments and records the verdict.
#[derive(Debug, Clone)]
pub struct AttachmentScan {
    /// As `taskforge_app`: this writes a tenant row, and the dispatcher's role
    /// bypasses row-level security and is granted on the outbox tables alone.
    pool: PgPool,
    store: Arc<dyn ObjectStore>,
    /// `None` when the deployment configured no scanner — see the module docs.
    scanner: Option<Arc<dyn Scanner>>,
}

impl AttachmentScan {
    #[must_use]
    pub const fn new(
        pool: PgPool,
        store: Arc<dyn ObjectStore>,
        scanner: Option<Arc<dyn Scanner>>,
    ) -> Self {
        Self {
            pool,
            store,
            scanner,
        }
    }
}

impl Consumer for AttachmentScan {
    fn name(&self) -> &'static str {
        NAME
    }

    async fn deliver(&self, event: &Claimed) -> Result<(), String> {
        if event.event_type != "attachment.uploaded" {
            return Ok(());
        }

        let Some(scanner) = self.scanner.as_ref() else {
            // Acknowledged, not retried: nothing about this delivery will
            // succeed later, and a delivery that fails forever is a dead-letter
            // alert about a deployment choice rather than a fault.
            tracing::warn!(
                attachment = %event.aggregate_id,
                "no scanner is configured, so this attachment stays unscanned and invisible \
                 (D-062: a deployment with no scanner fails closed)"
            );
            return Ok(());
        };

        let scope = WorkspaceScope::for_job(WorkspaceId::from_uuid(event.workspace_id));
        let mut tx = self
            .pool
            .begin()
            .await
            .map_err(|error| format!("the scan could not begin: {error}"))?;
        let mut scoped = Scoped::apply(&mut tx, &scope)
            .await
            .map_err(|error| format!("the scan could not scope: {error}"))?;

        // `find_for_commit`, reused rather than copied: it is the one query
        // that reads an attachment *before* it is visible, which is exactly what
        // a scanner needs, and a second function running the same SQL under a
        // different name would be a second place for the `deleted_at` guard to
        // drift. The key comes off the row rather than being rebuilt from its
        // parts, so `policy::object_key` stays the only thing that knows the
        // shape of a key.
        let Some(row) = attachment::find_for_commit(&mut scoped, event.aggregate_id)
            .await
            .map_err(|error| format!("reading the attachment failed: {error}"))?
        else {
            // Deleted between commit and scan. Nothing to do, and not an error.
            return Ok(());
        };

        // Read once, in full: a scanner has to see every byte, and `docs/28`
        // caps an attachment at 100 MB by default for exactly this reason —
        // that is the number the worker's memory is sized against.
        let bytes = self
            .store
            .read_prefix(
                &row.object_key,
                usize::try_from(row.byte_size).unwrap_or(usize::MAX),
            )
            .await
            .map_err(|error| format!("reading the object failed: {error}"))?;

        let verdict = scanner
            .scan(&bytes)
            .await
            .map_err(|error| format!("the scan did not complete: {error}"))?;

        let (status, detail) = match &verdict {
            Verdict::Clean => ("CLEAN", None),
            Verdict::Infected(signature) => ("INFECTED", Some(signature.as_str())),
        };

        attachment::mark_scanned(&mut scoped, row.id, status, detail)
            .await
            .map_err(|error| format!("recording the verdict failed: {error}"))?;

        tx.commit()
            .await
            .map_err(|error| format!("the scan could not commit: {error}"))?;

        // Deleted *after* the row says INFECTED, not before. The order matters:
        // if the delete succeeds and the commit does not, an attachment is
        // listed as clean with no object behind it. This way the worst case is
        // an orphaned object, which the sweeper already collects.
        if let Verdict::Infected(signature) = verdict {
            tracing::warn!(
                attachment = %row.id,
                %signature,
                "malware found in an attachment; removing the object"
            );
            if let Err(error) = self.store.delete(&row.object_key).await {
                tracing::error!(%error, "removing an infected object failed");
            }
        }

        Ok(())
    }
}
