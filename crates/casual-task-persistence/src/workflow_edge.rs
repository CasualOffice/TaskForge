//! The edges of a workflow: adding, editing and removing a transition.
//!
//! # Why this is not in `workflow_edit.rs`
//!
//! An edge and a status fail differently, and that is the whole split. Removing
//! a status has to move in-flight work, because tasks stand on statuses;
//! removing an edge moves nothing, because `docs/23` §Removing a transition is
//! explicit that "tasks are never *in* a transition, only in a status". One
//! module owns the migration machinery and the other owns none of it, so
//! keeping them together would put the dangerous code beside the free code and
//! invite a reader to assume they are equally careful.
//!
//! # `required_permission` is checked by a foreign key, not by a list here
//!
//! `workflow_transition.required_permission REFERENCES permission(key)`
//! (`migrations/0004`), and `docs/04` says the permission set is closed. An
//! unknown key therefore arrives as a foreign-key violation and becomes a `422`
//! naming the registry — and there is no list in this crate that could drift
//! from it, because there is no list.

use uuid::Uuid;

use crate::scoped::Scoped;
use crate::workflow_edit::WriteError;

/// A new edge. `from` of `None` is `docs/23`'s "from any status".
#[derive(Debug, Clone)]
pub struct NewTransition<'a> {
    pub from: Option<Uuid>,
    pub to: Uuid,
    pub required_permission: Option<&'a str>,
    pub required_fields: &'a [String],
    pub ignore_dependencies: bool,
}

/// Add an edge to a workflow.
///
/// # Errors
///
/// [`WriteError::Duplicate`] when the workflow already has this edge —
/// `workflow_transition_uq` is `NULLS NOT DISTINCT`, so a second "from any" edge
/// to the same target collides rather than silently duplicating.
/// [`WriteError::UnknownReference`] when `required_permission` is not a key in
/// the closed permission registry.
pub async fn insert_transition(
    scoped: &mut Scoped<'_>,
    workflow: Uuid,
    new: &NewTransition<'_>,
) -> Result<Uuid, WriteError> {
    let workspace = scoped.workspace_id().as_uuid();
    let id = Uuid::now_v7();
    sqlx::query(
        "INSERT INTO workflow_transition
             (id, workflow_id, workspace_id, from_status_id, to_status_id,
              required_permission, required_fields, ignore_dependencies)
         VALUES ($1,$2,$3,$4,$5,$6,$7,$8)",
    )
    .bind(id)
    .bind(workflow)
    .bind(workspace)
    .bind(new.from)
    .bind(new.to)
    .bind(new.required_permission)
    .bind(new.required_fields)
    .bind(new.ignore_dependencies)
    .execute(scoped.conn())
    .await?;
    Ok(id)
}

/// What a transition edit may change.
///
/// `from` and `to` are absent on purpose: changing either makes it a different
/// edge, and `docs/23` says removing a transition is free — so the honest
/// spelling is a delete and a create, which the unique index then checks.
#[derive(Debug, Clone, Default)]
pub struct TransitionPatch {
    /// `Some(None)` clears the requirement; `None` leaves it alone.
    pub required_permission: Option<Option<String>>,
    pub required_fields: Option<Vec<String>>,
    pub ignore_dependencies: Option<bool>,
}

/// Edit an edge's rules. Returns `false` when it is not in this workflow.
///
/// # Errors
///
/// [`WriteError::UnknownReference`] for a `required_permission` outside the
/// closed registry.
pub async fn update_transition(
    scoped: &mut Scoped<'_>,
    workflow: Uuid,
    transition: Uuid,
    patch: &TransitionPatch,
) -> Result<bool, WriteError> {
    // COALESCE against the bound parameter rather than a built statement: the
    // set of writable columns is fixed and small, and assembling SQL text per
    // patch is how a column name reaches a query from a request body.
    let affected = sqlx::query(
        "UPDATE workflow_transition
            SET required_permission = CASE WHEN $3 THEN $4 ELSE required_permission END,
                required_fields     = COALESCE($5, required_fields),
                ignore_dependencies = COALESCE($6, ignore_dependencies)
          WHERE id = $1 AND workflow_id = $2",
    )
    .bind(transition)
    .bind(workflow)
    .bind(patch.required_permission.is_some())
    .bind(patch.required_permission.clone().flatten())
    .bind(patch.required_fields.clone())
    .bind(patch.ignore_dependencies)
    .execute(scoped.conn())
    .await?
    .rows_affected();
    Ok(affected == 1)
}

/// Remove an edge. Returns `false` when it is not in this workflow.
///
/// `docs/23`: "allowed freely — it constrains future moves only. Tasks are
/// never in a transition, only in a status."
///
/// # Errors
///
/// Any database error.
pub async fn delete_transition(
    scoped: &mut Scoped<'_>,
    workflow: Uuid,
    transition: Uuid,
) -> Result<bool, sqlx::Error> {
    let affected =
        sqlx::query("DELETE FROM workflow_transition WHERE id = $1 AND workflow_id = $2")
            .bind(transition)
            .bind(workflow)
            .execute(scoped.conn())
            .await?
            .rows_affected();
    Ok(affected == 1)
}
