//! The core's own contributions.
//!
//! # The failure this prevents
//!
//! A seam nobody uses. `docs/34` opens with it: the core's own panels render
//! through the registry so that the contract is exercised in Phase 1 rather
//! than discovered to be wrong in Phase 3, when third parties depend on it.
//! Every panel, badge, tab, command and settings section TaskForge itself
//! ships is registered here as an ordinary [`Provider::Core`] contribution —
//! it gets no shortcut the vendor path does not also get.
//!
//! Phase 1 registers the frontend points only. The backend points exist in the
//! closed set and are contributed to by later phases (`docs/34` §Delivery); the
//! core does not route its own transitions or notifications through the plugin
//! contract, because those are the domain, not an extension of it.

use crate::contribution::{Contribution, Provider};
use crate::point::ExtensionPoint;
use crate::registry::{RegisterError, RegistryBuilder};

/// What the core contributes, as `(point, slug, title)`.
///
/// A table rather than a sequence of calls: the shape is data, and a reviewer
/// checking "is the drawer's activity panel still registered?" should be able
/// to answer it by reading one screen.
const CORE: &[(ExtensionPoint, &str, &str)] = &[
    // The task drawer, in the order it renders (docs/42 §Task drawer).
    (ExtensionPoint::UiTaskPanel, "details", "Details"),
    (ExtensionPoint::UiTaskPanel, "comments", "Comments"),
    (ExtensionPoint::UiTaskPanel, "attachments", "Attachments"),
    (ExtensionPoint::UiTaskPanel, "relations", "Relations"),
    (ExtensionPoint::UiTaskPanel, "activity", "Activity"),
    // Badges on cards and list rows, rendered from data already fetched.
    (ExtensionPoint::UiTaskBadge, "status", "Status"),
    (ExtensionPoint::UiTaskBadge, "priority", "Priority"),
    (ExtensionPoint::UiTaskBadge, "assignee", "Assignee"),
    (ExtensionPoint::UiTaskBadge, "due-date", "Due date"),
    // Project tabs.
    (ExtensionPoint::UiProjectTab, "board", "Board"),
    (ExtensionPoint::UiProjectTab, "list", "List"),
    (ExtensionPoint::UiProjectTab, "reports", "Reports"),
    // The palette. docs/42 §Command palette: create, navigate, assign,
    // transition, search — plus plugin-contributed entries later.
    (ExtensionPoint::UiCommand, "create-task", "Create task"),
    (ExtensionPoint::UiCommand, "go-to-project", "Go to project"),
    (ExtensionPoint::UiCommand, "assign", "Assign"),
    (ExtensionPoint::UiCommand, "transition", "Change status"),
    (ExtensionPoint::UiCommand, "search", "Search"),
    // Admin settings.
    (ExtensionPoint::UiSettingsSection, "members", "Members"),
    (ExtensionPoint::UiSettingsSection, "teams", "Teams"),
    (ExtensionPoint::UiSettingsSection, "roles", "Roles"),
    (ExtensionPoint::UiSettingsSection, "workflow", "Workflow"),
    (
        ExtensionPoint::UiSettingsSection,
        "extensions",
        "Extensions",
    ),
];

/// Register everything the core contributes.
///
/// # Errors
///
/// [`RegisterError`] if the core's own table has a duplicate or overflows a
/// point. Both are programming errors, surfaced rather than panicked so the
/// binary's start-up path decides how to die.
pub fn register_core(builder: &mut RegistryBuilder) -> Result<(), RegisterError> {
    for (point, slug, title) in CORE {
        let contribution = Contribution::new(*point, Provider::Core, slug, title)
            .expect("core contributions are checked by a test in this module");
        builder.register(contribution)?;
    }
    Ok(())
}

/// A registry holding exactly the core's contributions.
///
/// # Panics
///
/// If the core table is malformed — a build-time error that a test in this
/// module already catches, so reaching it means the test was deleted.
#[must_use]
pub fn core_registry() -> crate::registry::Registry {
    let mut builder = RegistryBuilder::new();
    register_core(&mut builder).expect("the core table is verified by tests");
    builder.build()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::point::Surface;

    #[test]
    fn the_core_table_is_well_formed() {
        // Exercises the same validation a vendor's manifest will meet, so a
        // rule that would refuse every real plugin is caught by our own data.
        for (point, slug, title) in CORE {
            Contribution::new(*point, Provider::Core, slug, title)
                .unwrap_or_else(|e| panic!("core contribution {slug} is invalid: {e}"));
        }
    }

    #[test]
    fn the_core_registers_without_a_duplicate_or_an_overflow() {
        let registry = core_registry();
        assert_eq!(registry.len(), CORE.len());
    }

    #[test]
    fn the_core_contributes_only_to_frontend_points_in_phase_1() {
        for contribution in core_registry().all() {
            assert_eq!(
                contribution.point().surface(),
                Surface::Frontend,
                "{} is a backend point; the core's domain logic must not route \
                 through the plugin contract (docs/34 §Delivery)",
                contribution.point()
            );
        }
    }

    #[test]
    fn every_frontend_point_is_exercised_by_the_core() {
        // This is the whole reason C-017 ships in Phase 1. A frontend point
        // with no core contribution is a contract nobody has ever run.
        for point in ExtensionPoint::ALL
            .iter()
            .filter(|p| p.surface() == Surface::Frontend)
        {
            assert!(
                !core_registry().at(*point).is_empty(),
                "{point} has no core contribution, so nothing exercises it \
                 before the first third-party plugin arrives"
            );
        }
    }

    #[test]
    fn the_core_holds_no_privilege_the_vendor_path_lacks() {
        // Registering a vendor contribution alongside the core's must work and
        // must not displace anything: `Provider::Core` is a label, not a tier.
        use crate::contribution::PluginId;
        let mut builder = RegistryBuilder::new();
        register_core(&mut builder).expect("core");
        let vendor = Provider::Plugin(PluginId::parse("com.example.qa").expect("valid"));
        builder
            .register(
                Contribution::new(
                    ExtensionPoint::UiTaskPanel,
                    vendor,
                    "signoff",
                    "QA sign-off",
                )
                .expect("valid"),
            )
            .expect("a vendor panel registers beside the core's");
        let registry = builder.build();
        assert_eq!(registry.len(), CORE.len() + 1);
        assert_eq!(registry.at(ExtensionPoint::UiTaskPanel).len(), 6);
    }
}
