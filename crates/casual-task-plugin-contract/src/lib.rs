//! # casual-task-plugin-contract
//!
//! The extension contract, versioned independently of the application (ADR-015).
//!
//! **Owns:** extension point definitions, manifest types and validation, the scope registry, and signature verification (`docs/34-PLUGIN-AND-EXTENSION-ARCHITECTURE.md`).
//!
//! **Must never own:** an execution host. Customer code never runs in either binary (ADR-016). This crate defines types and transport only.
//!
//! Boundary contract: `docs/19-WORKSPACE-SCAFFOLD-DESIGN.md`. An illegal
//! dependency here is a build failure, not a review comment.
//!
//! # What ships in Phase 1 (C-017)
//!
//! `docs/34` §Delivery: "Extension point registry, exercised by core panels
//! only". So this crate ships the closed set ([`point`]), who may contribute
//! and under what name ([`contribution`]), a build-once-then-frozen registry
//! ([`registry`]), the bounds and failure modes the host applies
//! ([`failure`]), and the core's own contributions going through exactly that
//! path ([`core_contributions`]).
//!
//! What it deliberately does **not** ship yet is the payload each point
//! carries — a panel's load URL, an action's handler. Phase 1 has no third
//! party to learn the shape from, and a compatibility contract guessed a
//! phase early is worse than one written a phase late.
//!
//! # The version of the contract, not of the app
//!
//! ADR-015: a manifest declares which contract version it targets, and that
//! number moves on its own schedule. [`CONTRACT_VERSION`] is that number. It
//! is not the crate version and not the release version.

pub mod contribution;
pub mod core_contributions;
pub mod failure;
pub mod point;
pub mod registry;

pub use contribution::{Contribution, PluginId, Provider};
pub use core_contributions::{core_registry, register_core};
pub use failure::{Bounds, OnFailure};
pub use point::{ExtensionPoint, Invocation, Surface};
pub use registry::{RegisterError, Registry, RegistryBuilder};

/// The extension contract version (ADR-015).
///
/// Major moves when a point is removed or its contract narrows — a plugin
/// built against the old major stops being installable. Minor moves when a
/// point is added or a contract widens compatibly. It is deliberately not
/// derived from `CARGO_PKG_VERSION`: tying it to the crate would move it on
/// every unrelated release and tell integrators nothing.
pub const CONTRACT_VERSION: (u16, u16) = (1, 0);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_contract_version_is_not_the_crate_version() {
        // If someone "simplifies" this to parse CARGO_PKG_VERSION, every app
        // release becomes a plugin-contract release and ADR-015 is undone.
        let crate_major: u16 = env!("CARGO_PKG_VERSION")
            .split('.')
            .next()
            .and_then(|s| s.parse().ok())
            .expect("a parseable crate version");
        assert_ne!(
            CONTRACT_VERSION.0, crate_major,
            "the contract version must move on its own schedule (ADR-015)"
        );
    }
}
