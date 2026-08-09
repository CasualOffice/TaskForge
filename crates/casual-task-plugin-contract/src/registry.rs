//! The registry itself: build once, freeze, then only read.
//!
//! # The failure this prevents
//!
//! A registry that can be written to at request time. Anything holding a
//! `&mut Registry` across an `await` serialises every request behind it, and
//! a contribution appearing halfway through a render is a bug nobody can
//! reproduce. So registration happens once, at start-up, through
//! [`RegistryBuilder`], which is *consumed* by [`RegistryBuilder::build`].
//! After that the registry is immutable and shareable — there is no
//! `&mut` method to call.

use std::collections::BTreeMap;

use crate::contribution::{Contribution, Provider};
use crate::point::ExtensionPoint;

/// Why a contribution could not be registered.
#[derive(Debug, PartialEq, Eq)]
pub enum RegisterError {
    /// Something is already registered under this key. Refused rather than
    /// overwritten: silently replacing a panel means an install can remove a
    /// feature the workspace was relying on, with no record that it happened.
    Duplicate(String),
    /// This point already holds [`Registry::MAX_PER_POINT`] contributions.
    /// Bounded per `docs/24` §D-040 — every bound names its overflow policy,
    /// and this one's is "refuse the newcomer", because dropping an existing
    /// contribution to make room silently disables working features.
    PointFull { point: ExtensionPoint, limit: usize },
}

impl core::fmt::Display for RegisterError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Duplicate(key) => write!(f, "{key} is already registered"),
            Self::PointFull { point, limit } => {
                write!(
                    f,
                    "{point} already holds the maximum of {limit} contributions"
                )
            }
        }
    }
}

impl std::error::Error for RegisterError {}

/// Accumulates contributions at start-up.
#[derive(Debug, Default)]
pub struct RegistryBuilder {
    // BTreeMap, not HashMap: iteration order is the registration order's
    // deterministic stand-in, so a panel list does not reshuffle between
    // restarts and a snapshot test is not flaky.
    by_key: BTreeMap<String, Contribution>,
}

impl RegistryBuilder {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// # Errors
    ///
    /// [`RegisterError::Duplicate`] if the key is taken, or
    /// [`RegisterError::PointFull`] if the point is at its limit.
    pub fn register(&mut self, contribution: Contribution) -> Result<(), RegisterError> {
        let key = contribution.key();
        if self.by_key.contains_key(&key) {
            return Err(RegisterError::Duplicate(key));
        }
        let point = contribution.point();
        let count = self.by_key.values().filter(|c| c.point() == point).count();
        if count >= Registry::MAX_PER_POINT {
            return Err(RegisterError::PointFull {
                point,
                limit: Registry::MAX_PER_POINT,
            });
        }
        self.by_key.insert(key, contribution);
        Ok(())
    }

    /// Freeze. Consuming `self` is the point: there is no path from a live
    /// [`Registry`] back to a mutable builder.
    #[must_use]
    pub fn build(self) -> Registry {
        let mut by_point: BTreeMap<&'static str, Vec<Contribution>> = BTreeMap::new();
        for contribution in self.by_key.into_values() {
            by_point
                .entry(contribution.point().name())
                .or_default()
                .push(contribution);
        }
        Registry { by_point }
    }
}

/// The frozen registry.
#[derive(Debug, Clone)]
pub struct Registry {
    by_point: BTreeMap<&'static str, Vec<Contribution>>,
}

impl Registry {
    /// A ceiling per point, so an install loop cannot make the task drawer
    /// unusable or the palette unreadable. Chosen well above any plausible
    /// real workspace, because a limit that real users hit is a bug report.
    pub const MAX_PER_POINT: usize = 64;

    /// Contributions to `point`, deterministically ordered.
    #[must_use]
    pub fn at(&self, point: ExtensionPoint) -> &[Contribution] {
        self.by_point
            .get(point.name())
            .map_or(&[], |v| v.as_slice())
    }

    /// Contributions to `point` from one provider.
    #[must_use]
    pub fn at_from(&self, point: ExtensionPoint, provider: &Provider) -> Vec<&Contribution> {
        self.at(point)
            .iter()
            .filter(|c| c.provider() == provider)
            .collect()
    }

    /// Every contribution, across every point.
    pub fn all(&self) -> impl Iterator<Item = &Contribution> {
        self.by_point.values().flatten()
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.by_point.values().map(Vec::len).sum()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.by_point.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::contribution::PluginId;

    fn core(point: ExtensionPoint, slug: &str) -> Contribution {
        Contribution::new(point, Provider::Core, slug, "Title").expect("valid")
    }

    #[test]
    fn a_duplicate_key_is_refused_rather_than_overwriting() {
        let mut builder = RegistryBuilder::new();
        builder
            .register(core(ExtensionPoint::UiTaskPanel, "summary"))
            .expect("first");
        let again = builder.register(core(ExtensionPoint::UiTaskPanel, "summary"));
        assert_eq!(
            again,
            Err(RegisterError::Duplicate(
                "core/ui.task.panel/summary".to_owned()
            )),
            "overwriting would let an install remove a feature with no record"
        );
    }

    #[test]
    fn a_point_is_bounded_and_the_newcomer_is_the_one_refused() {
        let mut builder = RegistryBuilder::new();
        for i in 0..Registry::MAX_PER_POINT {
            builder
                .register(core(ExtensionPoint::UiCommand, &format!("c{i}")))
                .expect("within the limit");
        }
        assert_eq!(
            builder.register(core(ExtensionPoint::UiCommand, "one-more")),
            Err(RegisterError::PointFull {
                point: ExtensionPoint::UiCommand,
                limit: Registry::MAX_PER_POINT,
            })
        );
        // The limit is per point, not global.
        builder
            .register(core(ExtensionPoint::UiTaskPanel, "still-fine"))
            .expect("a different point has its own budget");
    }

    #[test]
    fn ordering_is_stable_across_builds() {
        let build = || {
            let mut b = RegistryBuilder::new();
            for slug in ["zebra", "apple", "mango"] {
                b.register(core(ExtensionPoint::UiCommand, slug))
                    .expect("v");
            }
            b.build()
        };
        let first: Vec<String> = build()
            .at(ExtensionPoint::UiCommand)
            .iter()
            .map(|c| c.slug().to_owned())
            .collect();
        let second: Vec<String> = build()
            .at(ExtensionPoint::UiCommand)
            .iter()
            .map(|c| c.slug().to_owned())
            .collect();
        assert_eq!(first, second, "a reshuffling panel list is a flaky UI");
    }

    #[test]
    fn an_unpopulated_point_reads_as_empty_not_as_absent() {
        let registry = RegistryBuilder::new().build();
        assert!(registry.at(ExtensionPoint::UiTaskPanel).is_empty());
        assert!(registry.is_empty());
    }

    #[test]
    fn contributions_can_be_filtered_by_provider() {
        let vendor = Provider::Plugin(PluginId::parse("com.example.qa").expect("valid"));
        let mut builder = RegistryBuilder::new();
        builder
            .register(core(ExtensionPoint::UiTaskPanel, "summary"))
            .expect("v");
        builder
            .register(
                Contribution::new(ExtensionPoint::UiTaskPanel, vendor.clone(), "signoff", "QA")
                    .expect("valid"),
            )
            .expect("v");
        let registry = builder.build();
        assert_eq!(registry.at(ExtensionPoint::UiTaskPanel).len(), 2);
        assert_eq!(
            registry
                .at_from(ExtensionPoint::UiTaskPanel, &Provider::Core)
                .len(),
            1
        );
        assert_eq!(
            registry.at_from(ExtensionPoint::UiTaskPanel, &vendor).len(),
            1
        );
    }
}
