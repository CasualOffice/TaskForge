//! The extension point registry — a closed, versioned set (ADR-009).
//!
//! # The failure this prevents
//!
//! A hook system with no contract. `docs/34` §Alternatives names it: every
//! plugin couples to internals, and nothing can be refactored afterwards. The
//! set here is closed *in the type system*, so contributing to a point that was
//! never designed is a compile error rather than a support ticket in year two.
//!
//! # Why the set is duplicated against the design record
//!
//! `docs/34` §"The extension point registry" holds two markdown tables, and
//! adding a row to either is an ADR trigger. A table that drifts from the code
//! is worse than no table: it is a contract two teams read differently. So the
//! tables are parsed at test time and compared to [`ExtensionPoint::ALL`] in
//! both directions — a point added to code without an ADR fails, and a row
//! added to the document without an implementation fails too.

use core::fmt;

/// Which half of the system a point is contributed to.
///
/// A backend point runs in the API or worker process; a frontend point renders
/// in the browser. The distinction is not cosmetic — it decides which trust
/// tier and which failure mode apply (`docs/34` §"The four plugin classes").
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Surface {
    Backend,
    Frontend,
}

/// When the host calls a contribution.
///
/// This is part of the contract, not an implementation note: a contributor to
/// an `OnDomainEvent` point may be invoked with no user present and no request
/// to fail, and one at `InsideTransitionCheck` can block a person's work.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Invocation {
    /// User-initiated, synchronous, a person is waiting.
    OnDemand,
    /// Whenever the field is read or written.
    OnFieldAccess,
    /// On a domain event, after the transaction commits, asynchronously.
    OnDomainEvent,
    /// When a rule matches.
    OnRuleMatch,
    /// During notification fan-out.
    OnFanOut,
    /// Inside the transition's authorization path — this one can block work.
    InsideTransitionCheck,
    /// When a task is opened, lazily.
    OnTaskOpen,
    /// While rendering a list, from data already cached client-side.
    OnListRender,
    /// When a tab is selected, lazily.
    OnTabSelect,
    /// When the command palette opens.
    OnPaletteOpen,
    /// On navigation, lazily.
    OnNavigation,
}

/// The closed set of extension points (v1).
///
/// Every variant carries a stable wire name that outlives refactors — a
/// manifest written against `task.action` in Phase 3 must keep working when
/// the Rust variant is renamed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ExtensionPoint {
    // Backend.
    TaskAction,
    TaskField,
    AutomationTrigger,
    AutomationAction,
    EventSubscriber,
    NotificationChannel,
    ValidationTransition,
    // Frontend.
    UiTaskPanel,
    UiTaskBadge,
    UiProjectTab,
    UiCommand,
    UiSettingsSection,
}

impl ExtensionPoint {
    /// Every point, in the order `docs/34` lists them.
    ///
    /// Exhaustiveness is not merely convention: [`Self::name`] matches on
    /// `self` without a wildcard, so a new variant that is missing here still
    /// fails the round-trip test below rather than silently going unregistered.
    pub const ALL: &'static [Self] = &[
        Self::TaskAction,
        Self::TaskField,
        Self::AutomationTrigger,
        Self::AutomationAction,
        Self::EventSubscriber,
        Self::NotificationChannel,
        Self::ValidationTransition,
        Self::UiTaskPanel,
        Self::UiTaskBadge,
        Self::UiProjectTab,
        Self::UiCommand,
        Self::UiSettingsSection,
    ];

    /// The stable wire name. This is the string in a manifest; it is API.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::TaskAction => "task.action",
            Self::TaskField => "task.field",
            Self::AutomationTrigger => "automation.trigger",
            Self::AutomationAction => "automation.action",
            Self::EventSubscriber => "event.subscriber",
            Self::NotificationChannel => "notification.channel",
            Self::ValidationTransition => "validation.transition",
            Self::UiTaskPanel => "ui.task.panel",
            Self::UiTaskBadge => "ui.task.badge",
            Self::UiProjectTab => "ui.project.tab",
            Self::UiCommand => "ui.command",
            Self::UiSettingsSection => "ui.settings.section",
        }
    }

    /// Parse a wire name. Unknown names are `None` — a manifest naming a point
    /// this build does not have is refused at install, not at invocation.
    #[must_use]
    pub fn parse(name: &str) -> Option<Self> {
        Self::ALL.iter().copied().find(|p| p.name() == name)
    }

    #[must_use]
    pub const fn surface(self) -> Surface {
        match self {
            Self::TaskAction
            | Self::TaskField
            | Self::AutomationTrigger
            | Self::AutomationAction
            | Self::EventSubscriber
            | Self::NotificationChannel
            | Self::ValidationTransition => Surface::Backend,
            Self::UiTaskPanel
            | Self::UiTaskBadge
            | Self::UiProjectTab
            | Self::UiCommand
            | Self::UiSettingsSection => Surface::Frontend,
        }
    }

    #[must_use]
    pub const fn invocation(self) -> Invocation {
        match self {
            Self::TaskAction => Invocation::OnDemand,
            Self::TaskField => Invocation::OnFieldAccess,
            Self::AutomationTrigger | Self::EventSubscriber => Invocation::OnDomainEvent,
            Self::AutomationAction => Invocation::OnRuleMatch,
            Self::NotificationChannel => Invocation::OnFanOut,
            Self::ValidationTransition => Invocation::InsideTransitionCheck,
            Self::UiTaskPanel => Invocation::OnTaskOpen,
            Self::UiTaskBadge => Invocation::OnListRender,
            Self::UiProjectTab => Invocation::OnTabSelect,
            Self::UiCommand => Invocation::OnPaletteOpen,
            Self::UiSettingsSection => Invocation::OnNavigation,
        }
    }

    /// Whether a contribution to this point can prevent a user's action.
    ///
    /// Exactly one point can, by design, and `docs/34` §`validation.transition`
    /// spends a section on why. Callers that bound plugin latency read this
    /// rather than hard-coding the variant, so adding a second blocking point
    /// later cannot leave a call site unguarded.
    #[must_use]
    pub const fn can_block_user_action(self) -> bool {
        matches!(self, Self::ValidationTransition)
    }
}

impl fmt::Display for ExtensionPoint {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.name())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The rows of one of `docs/34`'s two point tables, as `(name, invoked)`.
    ///
    /// Deliberately a hand-rolled scan rather than a markdown parser: the
    /// dependency would be larger than the thing it parses, and the format is
    /// fixed by the same document.
    fn table_rows(heading: &str) -> Vec<String> {
        let doc = include_str!("../../../docs/34-PLUGIN-AND-EXTENSION-ARCHITECTURE.md");
        let after = doc
            .split(heading)
            .nth(1)
            .unwrap_or_else(|| panic!("docs/34 has a `{heading}` heading"));
        after
            .lines()
            .skip_while(|l| !l.starts_with('|'))
            .take_while(|l| l.starts_with('|'))
            // The header row and the `| --- |` separator are not points.
            .filter(|l| !l.contains("---") && !l.contains("| Point |"))
            .map(|l| l.split('|').nth(1).unwrap_or("").trim().replace('`', ""))
            .filter(|n| !n.is_empty())
            .collect()
    }

    #[test]
    fn the_closed_set_matches_the_design_record_in_both_directions() {
        let mut documented = table_rows("### Backend points");
        documented.extend(table_rows("### Frontend points"));

        let implemented: Vec<String> = ExtensionPoint::ALL
            .iter()
            .map(|p| p.name().to_owned())
            .collect();

        for name in &documented {
            assert!(
                implemented.contains(name),
                "docs/34 documents the extension point `{name}` but no ExtensionPoint \
                 variant implements it; a plugin written against the document would \
                 be refused at install with no explanation"
            );
        }
        for name in &implemented {
            assert!(
                documented.contains(name),
                "ExtensionPoint implements `{name}` but docs/34 does not list it; \
                 adding a point is an ADR trigger (ADR-009), not a code change"
            );
        }
    }

    #[test]
    fn the_documented_surface_decides_the_surface() {
        // Read from the document rather than restating the split here, so a
        // point that moves between the tables cannot keep the old surface.
        let backend = table_rows("### Backend points");
        let frontend = table_rows("### Frontend points");
        for point in ExtensionPoint::ALL {
            let name = point.name().to_owned();
            let expected = if backend.contains(&name) {
                Surface::Backend
            } else {
                assert!(frontend.contains(&name), "{name} is in neither table");
                Surface::Frontend
            };
            assert_eq!(point.surface(), expected, "wrong surface for {name}");
        }
    }

    #[test]
    fn wire_names_round_trip_and_are_unique() {
        let mut seen = std::collections::HashSet::new();
        for point in ExtensionPoint::ALL {
            assert_eq!(ExtensionPoint::parse(point.name()), Some(*point));
            assert!(seen.insert(point.name()), "duplicate name {point}");
        }
        assert_eq!(seen.len(), ExtensionPoint::ALL.len());
        assert_eq!(ExtensionPoint::parse("task.explode"), None);
    }

    #[test]
    fn exactly_one_point_can_block_a_users_action() {
        let blocking: Vec<_> = ExtensionPoint::ALL
            .iter()
            .filter(|p| p.can_block_user_action())
            .collect();
        assert_eq!(
            blocking,
            vec![&ExtensionPoint::ValidationTransition],
            "a second blocking point is a design decision with an ADR \
             (docs/34 §validation.transition), not an added `matches!` arm"
        );
    }
}
