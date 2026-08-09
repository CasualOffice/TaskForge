//! Roles, their permissions, and the grant that makes a workspace usable
//! (D-054).
//!
//! # The defect this closes
//!
//! `role_assignment` is the only source of authority in the system (migration
//! 0003: "No permission is granted anywhere else — not by a boolean column, not
//! by an `is_admin` flag, and not by project membership"). Nothing created one.
//! So a person could create a workspace, be its only member, and have no
//! authority in it at all: every write refused `403 TF-AZN-0001`, forever, with
//! no way to grant themselves anything because granting requires a grant.
//!
//! [`bootstrap`] is the answer. It materializes `docs/04`'s five role templates
//! into the workspace and assigns its creator the one that carries
//! `workspace.owner`, at `WORKSPACE` scope.
//!
//! # Why a handler cannot forget to call it
//!
//! [`crate::workspace::insert`] does not return a workspace. It returns an
//! [`crate::workspace::Unowned`], whose inner record is `pub(crate)` — so no
//! crate outside this one can open it, and the only thing in this crate that
//! does is [`bootstrap`]. A handler that creates a workspace and skips the
//! grant has nothing to build a response from and does not compile.
//!
//! That is the creation direction. The removal direction — revoking the last
//! owner afterwards — is refused by migration 0021's trigger, inside the
//! transaction, which is where `docs/04` control 4 says it belongs.

use casual_task_model::{Template, template};
use uuid::Uuid;

use crate::scoped::Scoped;
use crate::workspace::{Unowned, WorkspaceRecord};

/// What [`bootstrap`] created, for the audit record.
///
/// `docs/25` audits every grant with before/after. The grant made here has no
/// before, and this is its after.
#[derive(Debug, Clone)]
pub struct Bootstrap {
    /// The `role` rows written, `(id, name)`, in `docs/04` order.
    pub templates: Vec<(Uuid, String)>,
    /// The role the creator was granted.
    pub owner_role: Uuid,
    /// The `role_assignment` row.
    pub assignment: Uuid,
}

impl Bootstrap {
    /// The template names, for an activity record that has to read years later.
    #[must_use]
    pub fn template_names(&self) -> Vec<&str> {
        self.templates
            .iter()
            .map(|(_, name)| name.as_str())
            .collect()
    }
}

/// Seed the built-in role templates and grant `owner` the Owner role.
///
/// Runs in the caller's transaction, alongside the workspace row, its
/// membership row, and the `UnitOfWork::record` history — ADR-006: they commit
/// together or not at all. A workspace that committed without its owner grant
/// is the state this exists to make unreachable, and a separate transaction
/// would reintroduce a window in which it is real.
///
/// # Errors
///
/// Any database error. A unique violation means the workspace already has its
/// templates, which can only happen if this is called twice for one workspace.
pub async fn bootstrap(
    scoped: &mut Scoped<'_>,
    unowned: Unowned,
    owner: Uuid,
) -> Result<(WorkspaceRecord, Bootstrap), sqlx::Error> {
    let workspace = scoped.workspace_id().as_uuid();
    let mut templates = Vec::with_capacity(template::ROLES.len());

    for definition in template::ROLES {
        let id = insert_template(scoped, definition).await?;
        templates.push((id, definition.name.to_owned()));
    }

    // Found by permission, never by name (`casual_task_model::template::owner`):
    // renaming the template must not be able to bootstrap a workspace with a
    // role that does not own it.
    let owner_name = template::owner().name;
    let owner_role = templates
        .iter()
        .find(|(_, name)| name == owner_name)
        .map(|(id, _)| *id)
        .expect("every template in ROLES was just inserted");

    let assignment = Uuid::now_v7();
    sqlx::query(
        "INSERT INTO role_assignment
             (id, workspace_id, principal_type, principal_id, role_id,
              scope_type, scope_id, granted_by)
         VALUES ($1, $2, 'USER'::principal_type, $3, $4,
                 'WORKSPACE'::scope_type, $2, $3)",
    )
    .bind(assignment)
    .bind(workspace)
    .bind(owner)
    .bind(owner_role)
    .execute(scoped.conn())
    .await?;

    // `authz_epoch` is deliberately NOT bumped. docs/04 bumps it "by any grant
    // ... change" so that a cached decision cannot outlive it — but the cache
    // key is `(workspace_id, actor_id, project_id, authz_epoch)` and this
    // workspace did not exist when the transaction began. There is no entry to
    // invalidate, and bumping would only make the workspace's first epoch 2.

    Ok((
        unowned.into_record(),
        Bootstrap {
            templates,
            owner_role,
            assignment,
        },
    ))
}

/// One `role` row and its `role_permission` rows.
async fn insert_template(
    scoped: &mut Scoped<'_>,
    definition: &Template,
) -> Result<Uuid, sqlx::Error> {
    let workspace = scoped.workspace_id().as_uuid();
    let id = Uuid::now_v7();

    sqlx::query(
        "INSERT INTO role (id, workspace_id, name, is_template)
         VALUES ($1, $2, $3, true)",
    )
    .bind(id)
    .bind(workspace)
    .bind(definition.name)
    .execute(scoped.conn())
    .await?;

    // One statement for the whole set rather than one per permission: Owner
    // carries every key in the registry, and five templates at one round trip
    // each is 90-odd round trips inside the transaction that a user is waiting
    // on.
    let keys: Vec<String> = definition
        .permissions
        .iter()
        .map(|p| p.as_str().to_owned())
        .collect();
    sqlx::query(
        "INSERT INTO role_permission (role_id, permission)
         SELECT $1, unnest($2::text[])",
    )
    .bind(id)
    .bind(&keys)
    .execute(scoped.conn())
    .await?;

    Ok(id)
}

/// Whether this workspace has any `WORKSPACE`-scope grant carrying
/// `workspace.owner`.
///
/// The invariant D-054 is about, as a question anything can ask. Used by the
/// acceptance tests, and available to a future audit sweep.
///
/// # Errors
///
/// Any database error.
pub async fn has_owner(scoped: &mut Scoped<'_>) -> Result<bool, sqlx::Error> {
    let workspace = scoped.workspace_id().as_uuid();
    let found: Option<i32> = sqlx::query_scalar(
        "SELECT 1
           FROM role_assignment ra
           JOIN role_permission rp ON rp.role_id = ra.role_id
          WHERE ra.workspace_id = $1
            AND ra.scope_type = 'WORKSPACE'::scope_type
            AND rp.permission = 'workspace.owner'
          LIMIT 1",
    )
    .bind(workspace)
    .fetch_optional(scoped.conn())
    .await?;
    Ok(found.is_some())
}

/// The template roles in this workspace, `(name, permission count)`, by name.
///
/// # Errors
///
/// Any database error.
pub async fn templates(scoped: &mut Scoped<'_>) -> Result<Vec<(String, i64)>, sqlx::Error> {
    let workspace = scoped.workspace_id().as_uuid();
    sqlx::query_as(
        "SELECT r.name, count(rp.permission)
           FROM role r
           LEFT JOIN role_permission rp ON rp.role_id = r.id
          WHERE r.workspace_id = $1 AND r.is_template
          GROUP BY r.name
          ORDER BY r.name",
    )
    .bind(workspace)
    .fetch_all(scoped.conn())
    .await
}
