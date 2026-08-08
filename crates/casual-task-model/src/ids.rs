//! Sortable, typed identifiers. See `docs/22-DATABASE-SCHEMA.md` §Conventions.
//!
//! IDs are UUIDv7 — time-ordered, so they cluster well in a B-tree index and
//! double as a deterministic tiebreaker for cursor pagination
//! (`docs/26-SEARCH-INDEXING-AND-QUERY.md`).
//!
//! Each entity gets its own newtype. A `TaskId` cannot be passed where a
//! `ProjectId` is expected, which is the cheapest possible defence against the
//! class of bug where an ID is threaded through three layers into the wrong
//! query.

use serde::{Deserialize, Serialize};
use uuid::Uuid;

macro_rules! typed_id {
    ($(#[$m:meta])* $name:ident) => {
        $(#[$m])*
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
        #[serde(transparent)]
        pub struct $name(Uuid);

        impl $name {
            /// Allocate a new time-ordered identifier.
            #[allow(clippy::new_without_default)] // an ID has no meaningful default
            pub fn new() -> Self {
                Self(Uuid::now_v7())
            }

            pub fn from_uuid(id: Uuid) -> Self {
                Self(id)
            }

            pub fn as_uuid(&self) -> Uuid {
                self.0
            }
        }

        impl std::fmt::Display for $name {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                write!(f, "{}", self.0)
            }
        }
    };
}

typed_id!(/// Tenant boundary. Present on every tenant row.
          WorkspaceId);
typed_id!(/// A person. The one entity that spans workspaces.
          UserId);
typed_id!(TeamId);
typed_id!(ProjectId);
typed_id!(EnvironmentId);
typed_id!(MilestoneId);
typed_id!(TaskId);
typed_id!(TagId);
typed_id!(CommentId);
typed_id!(AttachmentId);
typed_id!(WorkflowId);
typed_id!(StatusId);
typed_id!(TransitionId);
typed_id!(RoleId);
typed_id!(RoleAssignmentId);
typed_id!(SavedViewId);
typed_id!(NotificationId);
typed_id!(AutomationRuleId);
typed_id!(PluginInstallationId);
typed_id!(EventId);
typed_id!(RequestId);
typed_id!(CorrelationId);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_are_time_ordered() {
        let a = TaskId::new();
        let b = TaskId::new();
        assert!(
            a < b,
            "UUIDv7 must sort by creation time; cursors depend on it"
        );
    }

    #[test]
    fn ids_are_unique() {
        let n = 1000;
        let set: std::collections::HashSet<_> = (0..n).map(|_| TaskId::new()).collect();
        assert_eq!(set.len(), n);
    }
}
