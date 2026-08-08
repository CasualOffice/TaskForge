//! The built-in role templates (`docs/04` §Built-in role templates).
//!
//! # This is data, not logic
//!
//! `docs/04`: "Templates are **cloneable starting points**, not special-cased
//! code. Cloning produces an ordinary custom role; nothing in the resolver
//! knows a role is 'built-in.'"
//!
//! Nothing here is read at authorization time. `casual-task-authz` resolves
//! over `role_assignment` rows and has no idea this module exists. It is read
//! exactly once per workspace, when the workspace is created, to write five
//! ordinary `role` rows and their `role_permission` rows. After that the
//! templates are indistinguishable from any role an admin authored.
//!
//! It lives beside the permission registry because a template *is* a named
//! subset of that registry, and the two have to be checked against each other:
//! [`ROLES`] is exhaustive over [`crate::permission::ALL`] by construction, so
//! a permission added to the registry cannot be silently left out of every
//! template.
//!
//! # Why the templates are per workspace and not seeded by a migration
//!
//! `role.workspace_id` is `NOT NULL REFERENCES workspace(id)` and the table
//! carries a row-level-security policy keyed on it (migration 0003, migration
//! 0010). A global template row cannot exist: it has no workspace to belong to,
//! and under the policy it would be invisible to everyone. So the templates are
//! materialized per workspace, in the transaction that creates the workspace.
//!
//! # What is derived, and what is a judgement call
//!
//! `docs/04` gives each template a one-line **shape**, not a permission set.
//! Two of the five follow from the words with nothing left over:
//!
//! - **Owner** — "Everything, including `workspace.delete` and billing."
//! - **Administrator** — "Everything except workspace deletion/transfer."
//!
//! The other three name capabilities in prose, and turning prose into a set of
//! 29 closed keys leaves genuinely undecided cells. Every one of them is
//! resolved in the **narrower** direction — AGENTS.md priority 1, "never grant
//! access that was not granted" — and every one is listed in
//! [`UNDECIDED`] rather than left for a reader to discover. `docs/04`
//! §Acceptance gates already names the artifact that settles them: the matrix
//! test, "every permission × every built-in role × every scope, as a golden
//! table". That is C-004, and it is not written yet. Tracked as **D-056**.

use crate::permission::{self, Permission};

/// A built-in role template: a name `docs/04` fixes, and the permissions it
/// carries.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Template {
    /// The name written into `role.name`. Fixed by `docs/04` — renaming one is
    /// a change to a name customers will have cloned.
    pub name: &'static str,
    pub permissions: &'static [Permission],
}

use permission as p;

/// Everything. `docs/04`: "Everything, including `workspace.delete` and
/// billing. Last one is protected."
///
/// Written as the registry itself rather than as a copy of it, so a permission
/// added to `docs/04`'s closed set is carried by Owner the moment it exists.
/// A new permission that no role can hold is a capability nobody can exercise.
const OWNER: &[Permission] = permission::ALL;

/// Everything except workspace deletion and transfer.
///
/// "Transfer" is `workspace.owner`: ownership is a grant, so transferring it
/// means granting it, and the registry has no other key that could mean it.
const ADMINISTRATOR: &[Permission] = &[
    p::WORKSPACE_MANAGE,
    p::PROJECT_CREATE,
    p::PROJECT_UPDATE,
    p::PROJECT_DELETE,
    p::PROJECT_MEMBER_MANAGE,
    p::PROJECT_ROLE_ASSIGN,
    p::PROJECT_WORKFLOW_MANAGE,
    p::TASK_READ,
    p::TASK_CREATE,
    p::TASK_UPDATE,
    p::TASK_ASSIGN,
    p::TASK_MOVE,
    p::TASK_TRANSITION,
    p::TASK_CLOSE,
    p::TASK_REOPEN,
    p::TASK_DELETE,
    p::TASK_COMMENT,
    p::TASK_HISTORY_READ,
    p::TASK_DEPENDENCY_OVERRIDE,
    p::TASK_ATTACHMENT_CREATE,
    p::TASK_ATTACHMENT_READ,
    p::TAG_MANAGE,
    p::ROLE_ASSIGN,
    p::ROLE_MANAGE,
    p::AUDIT_READ,
    p::PLUGIN_INSTALL,
    p::AUTOMATION_MANAGE,
];

/// "Full control within scoped projects: members, workflow, roles (under both
/// ceilings), all task actions."
///
/// Clause by clause: members → `project.member.manage`; workflow →
/// `project.workflow.manage`; roles → `project.role.assign`; all task actions →
/// every `task.*` key. `project.update` and `tag.manage` are "full control ...
/// within" the project.
///
/// `project.create` and `project.delete` are **withheld**: a grant at project
/// scope is a grant *inside* one project, and creating a sibling or deleting
/// the container are both acts on the workspace that holds it. See
/// [`UNDECIDED`].
const PROJECT_MANAGER: &[Permission] = &[
    p::PROJECT_UPDATE,
    p::PROJECT_MEMBER_MANAGE,
    p::PROJECT_ROLE_ASSIGN,
    p::PROJECT_WORKFLOW_MANAGE,
    p::TASK_READ,
    p::TASK_CREATE,
    p::TASK_UPDATE,
    p::TASK_ASSIGN,
    p::TASK_MOVE,
    p::TASK_TRANSITION,
    p::TASK_CLOSE,
    p::TASK_REOPEN,
    p::TASK_DELETE,
    p::TASK_COMMENT,
    p::TASK_HISTORY_READ,
    p::TASK_DEPENDENCY_OVERRIDE,
    p::TASK_ATTACHMENT_CREATE,
    p::TASK_ATTACHMENT_READ,
    p::TAG_MANAGE,
];

/// "Create/update/comment/transition tasks; read the project. No config, no
/// role assignment."
///
/// The four verbs map to `task.create`, `task.update`, `task.comment` and
/// `task.transition`. "Read the project" is `task.read`, and reading a task's
/// history and its attachments is reading it.
///
/// `task.close` and `task.reopen` are **withheld**, and that is the sharpest
/// cell in this file — see [`UNDECIDED`].
const MEMBER: &[Permission] = &[
    p::TASK_READ,
    p::TASK_CREATE,
    p::TASK_UPDATE,
    p::TASK_TRANSITION,
    p::TASK_COMMENT,
    p::TASK_HISTORY_READ,
    p::TASK_ATTACHMENT_CREATE,
    p::TASK_ATTACHMENT_READ,
];

/// "Read + comment on projects they are explicitly a member of."
///
/// Two verbs, two keys. The "explicitly a member of" half is **not** expressed
/// here: it is the `is_project_member` constraint, and `docs/04` puts
/// constraints on the *grant*, never on the role — "a constraint is a property
/// of a grant, never a restriction on other grants". A template that baked it
/// in would be describing an assignment it does not make.
const GUEST: &[Permission] = &[p::TASK_READ, p::TASK_COMMENT];

/// The five templates, in the order `docs/04` lists them.
pub const ROLES: &[Template] = &[
    Template {
        name: "Owner",
        permissions: OWNER,
    },
    Template {
        name: "Administrator",
        permissions: ADMINISTRATOR,
    },
    Template {
        name: "Project Manager",
        permissions: PROJECT_MANAGER,
    },
    Template {
        name: "Member",
        permissions: MEMBER,
    },
    Template {
        name: "Guest",
        permissions: GUEST,
    },
];

/// The template that carries `workspace.owner`, granted to a workspace's
/// creator (D-054).
///
/// Looked up by the permission rather than by the name, so renaming the
/// template cannot silently bootstrap a workspace with a role that does not own
/// it.
#[must_use]
pub fn owner() -> &'static Template {
    ROLES
        .iter()
        .find(|t| t.permissions.contains(&p::WORKSPACE_OWNER))
        .expect("one template carries workspace.owner; asserted below")
}

/// The cells `docs/04`'s prose does not decide, and the direction each was
/// resolved in.
///
/// Present as data rather than as a comment so it can be counted, printed, and
/// deleted one row at a time as C-004's golden matrix ratifies each. An empty
/// list means D-056 is closed.
pub const UNDECIDED: &[(&str, &str, &str)] = &[
    (
        "Member",
        "task.close / task.reopen",
        "docs/04 says \"transition tasks\"; docs/23 makes closing require \
         task.close IN ADDITION to a valid edge, so a Member who may transition \
         still cannot finish work. Withheld because the registry keeps them \
         separate from task.transition precisely so they can be, and widening a \
         template later is safe where narrowing it is not.",
    ),
    (
        "Member",
        "task.assign",
        "\"Create/update/comment/transition\" does not include assigning, and \
         task.assign is its own key. Withheld.",
    ),
    (
        "Project Manager",
        "project.delete",
        "\"Full control within scoped projects\" — deleting the project is an \
         act on the container, and the grant is scoped to what it would delete. \
         Withheld.",
    ),
    (
        "Guest",
        "task.history.read / task.attachment.read",
        "\"Read + comment\". Whether reading a task includes its history and \
         its attachments is undecided; both can carry content a guest was never \
         shown. Withheld.",
    ),
];

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeSet;

    fn keys(permissions: &[Permission]) -> BTreeSet<&'static str> {
        permissions.iter().map(Permission::as_str).collect()
    }

    #[test]
    fn the_five_templates_docs_04_names_are_the_five_that_exist() {
        // The names are what an admin clones and what the bootstrap looks up.
        // A sixth invented here would be a role docs/04 never described.
        let names: Vec<&str> = ROLES.iter().map(|t| t.name).collect();
        assert_eq!(
            names,
            vec![
                "Owner",
                "Administrator",
                "Project Manager",
                "Member",
                "Guest"
            ]
        );
    }

    #[test]
    fn owner_carries_every_permission_in_the_registry() {
        // docs/04: "Everything". Asserted against ALL rather than against a
        // copy, so a permission added to the closed set is carried by Owner
        // without anyone remembering to add it here.
        assert_eq!(keys(OWNER), keys(permission::ALL));
    }

    #[test]
    fn administrator_is_owner_minus_deletion_and_transfer() {
        // docs/04: "Everything except workspace deletion/transfer." Stated as a
        // difference, so a new permission lands in Administrator too and this
        // test names it if it should not have.
        let missing: BTreeSet<&str> = keys(OWNER)
            .difference(&keys(ADMINISTRATOR))
            .copied()
            .collect();
        assert_eq!(
            missing,
            BTreeSet::from(["workspace.delete", "workspace.owner"]),
            "Administrator differs from Owner by something other than deletion \
             and transfer of ownership"
        );
    }

    #[test]
    fn exactly_one_template_can_own_a_workspace() {
        // The bootstrap finds the owner template by permission, not by name. If
        // two carried workspace.owner, which one a new workspace got would
        // depend on declaration order.
        let owners: Vec<&str> = ROLES
            .iter()
            .filter(|t| t.permissions.contains(&p::WORKSPACE_OWNER))
            .map(|t| t.name)
            .collect();
        assert_eq!(owners, vec!["Owner"]);
        assert_eq!(owner().name, "Owner");
    }

    #[test]
    fn every_permission_a_template_names_exists_in_the_closed_registry() {
        // A template naming a key the registry does not have would fail at
        // INSERT against role_permission's foreign key — at runtime, in a
        // workspace create, for the user who happened to be first.
        let registry = keys(permission::ALL);
        for template in ROLES {
            for granted in keys(template.permissions) {
                assert!(
                    registry.contains(granted),
                    "{} carries {granted}, which is not in the registry",
                    template.name
                );
            }
        }
    }

    #[test]
    fn no_template_lists_a_permission_twice() {
        // A duplicate would violate role_permission's primary key and fail the
        // whole workspace create.
        for template in ROLES {
            assert_eq!(
                keys(template.permissions).len(),
                template.permissions.len(),
                "{} lists a permission twice",
                template.name
            );
        }
    }

    #[test]
    fn the_templates_narrow_monotonically() {
        // docs/04 describes them as a ladder: Owner ⊇ Administrator ⊇ Project
        // Manager ⊇ Member ⊇ Guest. Not required by the resolver — grants are
        // additive and independent — but a template that broke it would mean a
        // "narrower" role could do something a wider one could not, which is
        // the kind of surprise an admin cannot predict.
        for pair in ROLES.windows(2) {
            let (wider, narrower) = (&pair[0], &pair[1]);
            let extra: BTreeSet<&str> = keys(narrower.permissions)
                .difference(&keys(wider.permissions))
                .copied()
                .collect();
            assert!(
                extra.is_empty(),
                "{} carries {extra:?}, which {} does not",
                narrower.name,
                wider.name
            );
        }
    }

    #[test]
    fn every_undecided_cell_names_a_real_template_and_a_real_permission() {
        // D-056's list is deleted a row at a time as C-004 ratifies each. A row
        // naming a template or a key that no longer exists would be a decision
        // nobody can act on.
        let registry = keys(permission::ALL);
        for (role, keys_named, reason) in UNDECIDED {
            assert!(
                ROLES.iter().any(|t| t.name == *role),
                "{role} is not a template"
            );
            for key in keys_named.split('/').map(str::trim) {
                assert!(registry.contains(key), "{key} is not in the registry");
            }
            assert!(
                reason.len() > 40,
                "{role}/{keys_named} has no stated reason"
            );
        }
    }

    #[test]
    fn the_undecided_cells_are_all_withheld_rather_than_granted() {
        // The direction matters more than the decision. AGENTS.md priority 1:
        // never grant access that was not granted. Widening a template later is
        // an additive change; narrowing one takes away authority someone is
        // already using.
        for (role, keys_named, _) in UNDECIDED {
            let template = ROLES
                .iter()
                .find(|t| t.name == *role)
                .expect("checked above");
            for key in keys_named.split('/').map(str::trim) {
                assert!(
                    !keys(template.permissions).contains(key),
                    "{role} was recorded as undecided about {key} and grants it \
                     anyway — an undecided cell must fail closed"
                );
            }
        }
    }
}
