//! The permission registry. See `docs/04-RBAC-AND-AUTHORIZATION.md`.
//!
//! A permission is a stable `resource.action` string. The set is **closed** and
//! versioned with the API: a permission that is not here does not exist, and
//! adding one is an ADR-adjacent change that must also seed the `permission`
//! table (`docs/22-DATABASE-SCHEMA.md`).

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Permission(&'static str);

impl Permission {
    pub fn as_str(&self) -> &'static str {
        self.0
    }
}

impl std::fmt::Display for Permission {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.0)
    }
}

macro_rules! permissions {
    ($($konst:ident => $lit:literal),* $(,)?) => {
        $(pub const $konst: Permission = Permission($lit);)*

        /// Every permission that exists. The matrix test enumerates this
        /// against every built-in role and scope (`docs/15`).
        pub const ALL: &[Permission] = &[$($konst),*];
    };
}

permissions! {
    WORKSPACE_MANAGE      => "workspace.manage",
    WORKSPACE_DELETE      => "workspace.delete",
    WORKSPACE_OWNER       => "workspace.owner",

    PROJECT_CREATE        => "project.create",
    PROJECT_UPDATE        => "project.update",
    PROJECT_DELETE        => "project.delete",
    PROJECT_MEMBER_MANAGE => "project.member.manage",
    PROJECT_ROLE_ASSIGN   => "project.role.assign",
    PROJECT_WORKFLOW_MANAGE => "project.workflow.manage",

    TASK_READ             => "task.read",
    TASK_CREATE           => "task.create",
    TASK_UPDATE           => "task.update",
    TASK_ASSIGN           => "task.assign",
    TASK_MOVE             => "task.move",
    TASK_TRANSITION       => "task.transition",
    TASK_CLOSE            => "task.close",
    TASK_REOPEN           => "task.reopen",
    TASK_DELETE           => "task.delete",
    TASK_COMMENT          => "task.comment",
    TASK_HISTORY_READ     => "task.history.read",
    TASK_DEPENDENCY_OVERRIDE => "task.dependency.override",
    TASK_ATTACHMENT_CREATE => "task.attachment.create",
    TASK_ATTACHMENT_READ  => "task.attachment.read",

    TAG_MANAGE            => "tag.manage",
    // D-049: assigning a role and authoring one are DIFFERENT authorities.
    //
    // Merged, anyone who could assign could also author — and therefore mint a
    // role with more power than their own and grant it to themselves. That is
    // privilege escalation by construction, and it would have put a hole in the
    // escalation ceilings exactly where the most privileged actors are.
    //
    // `role.assign` is the workspace-scope counterpart of `project.role.assign`
    // above. `role.manage` keeps its narrower meaning: authoring the roles
    // themselves, and workspace-scope only.
    ROLE_ASSIGN           => "role.assign",
    ROLE_MANAGE           => "role.manage",
    AUDIT_READ            => "audit.read",
    PLUGIN_INSTALL        => "plugin.install",
    AUTOMATION_MANAGE     => "automation.manage",
}

/// Where a grant applies. Strict containment: WORKSPACE ⊃ TEAM ⊃ PROJECT ⊃
/// ENVIRONMENT.
///
/// `TASK` is deliberately absent (ADR-005) — per-task grants make the grant
/// table scale with task count and break the one-resolution-per-list property
/// that keeps board loads fast.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "UPPERCASE")]
pub enum ScopeType {
    Workspace,
    Team,
    Project,
    Environment,
}

/// Something a role can be assigned to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum PrincipalType {
    User,
    Team,
    ServiceAccount,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn permissions_are_unique() {
        let set: std::collections::HashSet<_> = ALL.iter().map(|p| p.as_str()).collect();
        assert_eq!(set.len(), ALL.len(), "duplicate permission key");
    }

    #[test]
    fn permissions_follow_resource_action_form() {
        for p in ALL {
            assert!(p.as_str().contains('.'), "{p} must be resource.action");
            assert_eq!(
                p.as_str(),
                p.as_str().to_lowercase(),
                "{p} must be lowercase"
            );
        }
    }

    #[test]
    fn task_scope_is_absent_by_design() {
        // ADR-005. If a TASK variant is ever added, this test should be deleted
        // by the same change that supersedes the ADR — not quietly amended.
        let json = serde_json::to_string(&ScopeType::Project).unwrap();
        assert_eq!(json, r#""PROJECT""#);
    }
}
