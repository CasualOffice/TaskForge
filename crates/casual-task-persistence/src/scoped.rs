//! The scoped connection — the only door to tenant data.
//!
//! `docs/32` makes tenancy two independent mechanisms that must **both** fail
//! to leak: a [`WorkspaceScope`] that only auth middleware can mint, and
//! PostgreSQL row-level security behind it. This module is where the two meet.
//!
//! # Why a type rather than a convention
//!
//! Migration 0010's policy compares `workspace_id` against
//! `current_setting('taskforge.workspace_id', true)`, and an unscoped session
//! sees `NULL` — so it returns **no rows** rather than every row. That is the
//! right direction to fail, but it means a repository that forgets to set the
//! GUC does not blow up: it quietly finds nothing, which reads as "no data"
//! rather than "bug".
//!
//! [`Scoped`] removes the chance to forget. A repository takes one, and a
//! `Scoped` cannot exist without a `WorkspaceScope` having been applied to the
//! connection it wraps.
//!
//! # Why the setting is transaction-local
//!
//! `set_config(..., true)` is scoped to the transaction, so it is reset when
//! the transaction ends. On a pooled connection that matters enormously: a
//! session-level setting would outlive the request and the next checkout would
//! inherit another tenant's scope. The schema gate asserts the pooled case
//! ("no pool bleed after COMMIT"); this is the code that makes it true.

use casual_task_model::{WorkspaceId, WorkspaceScope};
use sqlx::{PgConnection, Postgres, Transaction};

/// The GUC migration 0010's policy reads.
const SCOPE_SETTING: &str = "taskforge.workspace_id";

/// A transaction that has had a tenant scope applied to it.
///
/// Borrowed rather than owned so a repository cannot commit somebody else's
/// transaction: the caller owns the unit of work, which is what lets one
/// transaction span the domain write, its activity row, its audit row and its
/// outbox row — the guarantee `docs/25` rests on.
#[derive(Debug)]
pub struct Scoped<'t> {
    tx: &'t mut Transaction<'static, Postgres>,
    workspace: WorkspaceId,
}

impl<'t> Scoped<'t> {
    /// Apply `scope` to `tx` and return the scoped handle.
    ///
    /// # Errors
    ///
    /// Any error setting the GUC. A failure here must abort the unit of work
    /// rather than continue unscoped — an unscoped transaction sees nothing,
    /// so continuing would look like an empty result rather than a fault.
    pub async fn apply(
        tx: &'t mut Transaction<'static, Postgres>,
        scope: &WorkspaceScope,
    ) -> Result<Self, sqlx::Error> {
        let workspace = scope.id();
        // `true` is the local flag: transaction-scoped, reset at COMMIT or
        // ROLLBACK. Bound as a parameter rather than formatted, so a workspace
        // id can never be interpolated into SQL text.
        sqlx::query("SELECT set_config($1, $2, true)")
            .bind(SCOPE_SETTING)
            .bind(workspace.as_uuid().to_string())
            .execute(&mut **tx)
            .await?;
        Ok(Self { tx, workspace })
    }

    /// The workspace this handle is scoped to.
    ///
    /// Repositories that need to write `workspace_id` on an INSERT take it from
    /// here rather than from a parameter, so the row written and the policy
    /// enforced can never disagree.
    pub fn workspace_id(&self) -> WorkspaceId {
        self.workspace
    }

    /// The underlying connection, already scoped.
    pub fn conn(&mut self) -> &mut PgConnection {
        self.tx
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_setting_name_matches_the_policy() {
        // Migration 0010 formats this name into every policy it creates. If the
        // two drift, every scoped query silently returns nothing — the failure
        // this constant exists to prevent, and one that no query would report.
        let migration = include_str!("../../../migrations/0010_row_level_security.sql");
        // The exact doubled-quoted form the policy is built with, not a
        // substring match: `contains(SCOPE_SETTING)` would still pass if the
        // constant were truncated to `taskforge.workspace`, since that is a
        // prefix of the real name. A near-miss is precisely the drift worth
        // catching, and it is the one a loose assertion misses.
        let quoted = format!("''{SCOPE_SETTING}''");
        assert!(
            migration.contains(&quoted),
            "migration 0010 builds its policy with a different setting name than \
             `{SCOPE_SETTING}`. Every scoped read would return zero rows without \
             erroring — the policy and the code that satisfies it have drifted."
        );
    }
}
