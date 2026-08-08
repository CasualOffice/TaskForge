//! Workflow storage, and the default workflow every workspace gets (C-007).
//!
//! # Why a project create provisions this
//!
//! `project.workflow_id` is `NOT NULL` and `docs/23` says the default workflow
//! "works with zero configuration. Most teams never change it." Between those
//! two facts sits a workspace that has never had a workflow created, and a
//! project create that cannot succeed. So the first create in a workspace
//! materializes the workflow `docs/23` §The default workflow draws, in the same
//! transaction as the project.
//!
//! The race — two concurrent creates in a fresh workspace — is closed by a
//! partial unique index (migration 0019), not by a check-then-insert. The
//! loser's `INSERT` fails, it re-reads, and both projects end up on one default
//! workflow. Making it impossible in the schema is cheaper than making it
//! unlikely in the code.
//!
//! # This module stores a workflow; it does not run one
//!
//! Validation lives in `casual-task-workflow` and composition in
//! `casual-task-app`. What is here is rows in and rows out.

use uuid::Uuid;

use crate::scoped::Scoped;

/// A `workflow_status` row.
#[derive(Debug, Clone)]
pub struct StatusRow {
    pub id: Uuid,
    pub name: String,
    /// One of the five permanent states (`docs/23`).
    pub state: String,
    pub position: i32,
    pub is_initial: bool,
}

/// The tuple `workflow_transition` decodes into, before it becomes a
/// [`TransitionRow`].
type TransitionTuple = (Uuid, Option<Uuid>, Uuid, Option<String>, Vec<String>, bool);

/// A `workflow_transition` row. `from` of `None` is "from any status".
#[derive(Debug, Clone)]
pub struct TransitionRow {
    pub id: Uuid,
    pub from: Option<Uuid>,
    pub to: Uuid,
    pub required_permission: Option<String>,
    pub required_fields: Vec<String>,
    pub ignore_dependencies: bool,
}

/// The default workflow's statuses, exactly as `docs/23` draws them.
///
/// `(name, state, position, is_initial)`. `Blocked` is `ACTIVE` and not a state
/// of its own — `docs/23`: "blocked work is committed work whose clock is still
/// running".
const DEFAULT_STATUSES: &[(&str, &str, i32, bool)] = &[
    ("Backlog", "BACKLOG", 1, true),
    ("Todo", "PLANNED", 2, false),
    ("In Progress", "ACTIVE", 3, false),
    ("Blocked", "ACTIVE", 4, false),
    ("Done", "COMPLETED", 5, false),
    ("Canceled", "CANCELED", 6, false),
];

/// The default workflow's edges: `(from, to, required_permission)`.
///
/// `from` of `None` is the wildcard `docs/23` uses for "Cancel from anywhere".
/// `task.close` and `task.reopen` sit on the edges rather than being special
/// cases in code — `docs/23` §Closing and reopening: closing "requires
/// `task.close` **and** a valid transition edge; both, not either", and an edge
/// that carries its own permission is how "both" is expressed with one
/// mechanism.
const DEFAULT_TRANSITIONS: &[(Option<&str>, &str, Option<&str>)] = &[
    (Some("Backlog"), "Todo", None),
    (Some("Todo"), "Backlog", None),
    (Some("Todo"), "In Progress", None),
    (Some("In Progress"), "Todo", None),
    (Some("In Progress"), "Blocked", None),
    (Some("Blocked"), "In Progress", None),
    (Some("In Progress"), "Done", Some("task.close")),
    (Some("Done"), "In Progress", Some("task.reopen")),
    (None, "Canceled", None),
];

/// The workspace's default workflow, or `None` when it has none yet.
///
/// # Errors
///
/// Any database error.
pub async fn default_workflow(scoped: &mut Scoped<'_>) -> Result<Option<Uuid>, sqlx::Error> {
    let workspace = scoped.workspace_id().as_uuid();
    sqlx::query_scalar("SELECT id FROM workflow WHERE workspace_id = $1 AND is_default")
        .bind(workspace)
        .fetch_optional(scoped.conn())
        .await
}

/// The workspace's default workflow, creating it if it does not exist.
///
/// # Errors
///
/// Any database error other than the unique violation this resolves.
pub async fn ensure_default_workflow(scoped: &mut Scoped<'_>) -> Result<Uuid, sqlx::Error> {
    if let Some(existing) = default_workflow(scoped).await? {
        return Ok(existing);
    }
    // A unique violation here means a concurrent create in a fresh workspace
    // won the race (migration 0019). It is not recoverable *inside* this
    // transaction — the winner has not committed, so this transaction cannot
    // see its row — so the error propagates and the client's retry succeeds
    // against the committed workflow. Two default workflows never exist.
    create_default_workflow(scoped).await
}

async fn create_default_workflow(scoped: &mut Scoped<'_>) -> Result<Uuid, sqlx::Error> {
    let workspace = scoped.workspace_id().as_uuid();
    let workflow = Uuid::now_v7();

    sqlx::query(
        "INSERT INTO workflow (id, workspace_id, name, is_default)
         VALUES ($1,$2,'Default',true)",
    )
    .bind(workflow)
    .bind(workspace)
    .execute(scoped.conn())
    .await?;

    let mut ids: Vec<(&str, Uuid)> = Vec::with_capacity(DEFAULT_STATUSES.len());
    for (name, state, position, is_initial) in DEFAULT_STATUSES {
        let id = Uuid::now_v7();
        sqlx::query(
            "INSERT INTO workflow_status
                 (id, workflow_id, workspace_id, name, state, position, is_initial)
             VALUES ($1,$2,$3,$4,$5::task_state,$6,$7)",
        )
        .bind(id)
        .bind(workflow)
        .bind(workspace)
        .bind(*name)
        .bind(*state)
        .bind(*position)
        .bind(*is_initial)
        .execute(scoped.conn())
        .await?;
        ids.push((name, id));
    }
    let id_of = |name: &str| {
        ids.iter()
            .find(|(n, _)| *n == name)
            .map(|(_, id)| *id)
            .expect("DEFAULT_TRANSITIONS only names statuses in DEFAULT_STATUSES")
    };

    for (from, to, permission) in DEFAULT_TRANSITIONS {
        sqlx::query(
            "INSERT INTO workflow_transition
                 (id, workflow_id, workspace_id, from_status_id, to_status_id,
                  required_permission, ignore_dependencies)
             VALUES ($1,$2,$3,$4,$5,$6,$7)",
        )
        .bind(Uuid::now_v7())
        .bind(workflow)
        .bind(workspace)
        .bind(from.map(id_of))
        .bind(id_of(to))
        .bind(*permission)
        // `docs/23`: Cancel is reachable from anywhere and opts out of
        // dependency gating. Every other edge gates.
        .bind(from.is_none())
        .execute(scoped.conn())
        .await?;
    }

    Ok(workflow)
}

/// Every status and transition of one workflow.
///
/// # Errors
///
/// Any database error.
pub async fn load(
    scoped: &mut Scoped<'_>,
    workflow: Uuid,
) -> Result<(Vec<StatusRow>, Vec<TransitionRow>), sqlx::Error> {
    let statuses: Vec<(Uuid, String, String, i32, bool)> = sqlx::query_as(
        "SELECT id, name, state::text, position, is_initial
           FROM workflow_status
          WHERE workflow_id = $1
          ORDER BY position",
    )
    .bind(workflow)
    .fetch_all(scoped.conn())
    .await?;

    let transitions: Vec<TransitionTuple> = sqlx::query_as(
        "SELECT id, from_status_id, to_status_id, required_permission,
                    required_fields, ignore_dependencies
               FROM workflow_transition
              WHERE workflow_id = $1",
    )
    .bind(workflow)
    .fetch_all(scoped.conn())
    .await?;

    Ok((
        statuses
            .into_iter()
            .map(|(id, name, state, position, is_initial)| StatusRow {
                id,
                name,
                state,
                position,
                is_initial,
            })
            .collect(),
        transitions
            .into_iter()
            .map(
                |(id, from, to, required_permission, required_fields, ignore_dependencies)| {
                    TransitionRow {
                        id,
                        from,
                        to,
                        required_permission,
                        required_fields,
                        ignore_dependencies,
                    }
                },
            )
            .collect(),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_default_workflow_has_exactly_one_initial_status() {
        // Migration 0004 enforces it with a partial unique index, so a second
        // one would fail at INSERT. Catching it here names the actual problem.
        assert_eq!(
            DEFAULT_STATUSES.iter().filter(|s| s.3).count(),
            1,
            "docs/23: exactly one status per workflow is initial"
        );
    }

    #[test]
    fn every_edge_names_a_status_the_workflow_has() {
        for (from, to, _) in DEFAULT_TRANSITIONS {
            for name in from.iter().chain(std::iter::once(to)) {
                assert!(
                    DEFAULT_STATUSES.iter().any(|(n, ..)| n == name),
                    "{name} is not a status of the default workflow"
                );
            }
        }
    }

    #[test]
    fn every_state_the_default_workflow_uses_is_one_of_the_five() {
        // docs/23: the five states are the permanent API contract. A typo here
        // would be caught by the enum at INSERT, but only at runtime.
        for (name, state, ..) in DEFAULT_STATUSES {
            assert!(
                casual_task_model::TaskState::ALL
                    .iter()
                    .any(|s| serde_json::to_string(s).unwrap_or_default() == format!("\"{state}\"")),
                "{name} maps to {state}, which is not one of the five states"
            );
        }
    }

    #[test]
    fn cancel_is_the_only_wildcard_edge_and_the_only_one_that_ignores_blockers() {
        // docs/23 draws exactly one "from any status" edge, and ADR-019 gates
        // every other transition on unresolved blockers.
        let wildcards: Vec<_> = DEFAULT_TRANSITIONS
            .iter()
            .filter(|(from, ..)| from.is_none())
            .collect();
        assert_eq!(wildcards.len(), 1);
        assert_eq!(wildcards[0].1, "Canceled");
    }

    #[test]
    fn closing_and_reopening_carry_their_permissions_on_the_edge() {
        // docs/23 §Closing and reopening. Without these the default workflow
        // would let anyone who may transition also close and reopen, which is
        // the distinction the two permissions exist to make.
        let permission_for = |to: &str| {
            DEFAULT_TRANSITIONS
                .iter()
                .find(|(_, t, _)| *t == to)
                .and_then(|(_, _, p)| *p)
        };
        assert_eq!(permission_for("Done"), Some("task.close"));
        assert_eq!(
            DEFAULT_TRANSITIONS
                .iter()
                .find(|(from, to, _)| *from == Some("Done") && *to == "In Progress")
                .and_then(|(_, _, p)| *p),
            Some("task.reopen")
        );
    }
}
