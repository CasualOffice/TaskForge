//! What a provider contributes to a point, and who provided it.
//!
//! # The failure this prevents
//!
//! Core features that special-case themselves. `docs/34` opens by saying the
//! core's own panels must render through the registry, because a seam only the
//! plugins use is a seam nobody has tested — "and we find that out in Phase 1,
//! not Phase 3". So [`Provider::Core`] exists and is an ordinary provider: the
//! registry gives it no privilege, and removing plugins from the picture does
//! not remove the indirection.

use core::fmt;

use crate::point::ExtensionPoint;

/// A reverse-DNS plugin identifier, immutable once published (`docs/34`
/// §"The manifest").
///
/// Validated on construction rather than on use: an id that reaches the
/// registry has already been checked, so no call site has to re-derive the
/// rules and get them subtly different.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct PluginId(String);

/// Why a plugin id was refused.
#[derive(Debug, PartialEq, Eq)]
pub enum PluginIdError {
    Empty,
    /// Not reverse-DNS — fewer than two dot-separated labels.
    NotReverseDns,
    /// A label was empty (`com..thing`), or the id started or ended with a dot.
    EmptyLabel,
    /// A character outside `[a-z0-9-]`. Uppercase is refused rather than
    /// lowercased: two ids differing only in case would be one plugin to a
    /// case-insensitive registry and two to this one.
    IllegalCharacter(char),
    /// Longer than [`PluginId::MAX_LENGTH`].
    TooLong,
}

impl fmt::Display for PluginIdError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Empty => f.write_str("plugin id is empty"),
            Self::NotReverseDns => f.write_str("plugin id is not reverse-DNS (needs a dot)"),
            Self::EmptyLabel => f.write_str("plugin id has an empty label"),
            Self::IllegalCharacter(c) => write!(f, "plugin id contains {c:?}; allowed: a-z 0-9 -"),
            Self::TooLong => write!(f, "plugin id exceeds {} characters", PluginId::MAX_LENGTH),
        }
    }
}

impl std::error::Error for PluginIdError {}

impl PluginId {
    /// Bounded so an id cannot be used to bloat every audit row that names it.
    pub const MAX_LENGTH: usize = 128;

    /// # Errors
    ///
    /// [`PluginIdError`] if the id is not a well-formed reverse-DNS name.
    pub fn parse(raw: &str) -> Result<Self, PluginIdError> {
        if raw.is_empty() {
            return Err(PluginIdError::Empty);
        }
        if raw.len() > Self::MAX_LENGTH {
            return Err(PluginIdError::TooLong);
        }
        if let Some(c) = raw
            .chars()
            .find(|c| !(c.is_ascii_lowercase() || c.is_ascii_digit() || *c == '-' || *c == '.'))
        {
            return Err(PluginIdError::IllegalCharacter(c));
        }
        let labels: Vec<&str> = raw.split('.').collect();
        if labels.len() < 2 {
            return Err(PluginIdError::NotReverseDns);
        }
        if labels.iter().any(|l| l.is_empty()) {
            return Err(PluginIdError::EmptyLabel);
        }
        Ok(Self(raw.to_owned()))
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for PluginId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// Who contributed.
///
/// `Core` is not a privilege level. It exists so that the host can say, in an
/// error message or an audit row, whose code failed — and so a panel shipped by
/// TaskForge and a panel shipped by a vendor travel the same path.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Provider {
    Core,
    Plugin(PluginId),
}

impl fmt::Display for Provider {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Core => f.write_str("core"),
            Self::Plugin(id) => f.write_str(id.as_str()),
        }
    }
}

/// One registered contribution.
///
/// The payload a point needs — a panel's lazy-load URL, an action's label — is
/// deliberately *not* here in v1. Phase 1 ships the registry exercised by core
/// panels only (`docs/34` §Delivery), and a payload union invented before the
/// first remote plugin exists would be a compatibility contract guessed rather
/// than learned. What is here is the part every point shares and that later
/// phases cannot change: who, where, and under what name.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Contribution {
    point: ExtensionPoint,
    provider: Provider,
    slug: String,
    title: String,
}

/// Why a contribution was refused.
#[derive(Debug, PartialEq, Eq)]
pub enum ContributionError {
    /// The slug was empty, over-long, or not `[a-z0-9-]`.
    BadSlug(String),
    /// The title was empty or over-long.
    BadTitle(String),
}

impl fmt::Display for ContributionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::BadSlug(why) | Self::BadTitle(why) => f.write_str(why),
        }
    }
}

impl std::error::Error for ContributionError {}

impl Contribution {
    /// A slug is bounded so a registry key cannot grow without limit, and kept
    /// to `[a-z0-9-]` so it is safe in a URL fragment, a CSS class, and a log
    /// line without three different escaping rules.
    pub const MAX_SLUG_LENGTH: usize = 64;
    pub const MAX_TITLE_LENGTH: usize = 120;

    /// # Errors
    ///
    /// [`ContributionError`] if the slug or title is malformed.
    pub fn new(
        point: ExtensionPoint,
        provider: Provider,
        slug: &str,
        title: &str,
    ) -> Result<Self, ContributionError> {
        if slug.is_empty() {
            return Err(ContributionError::BadSlug("slug is empty".into()));
        }
        if slug.len() > Self::MAX_SLUG_LENGTH {
            return Err(ContributionError::BadSlug(format!(
                "slug exceeds {} characters",
                Self::MAX_SLUG_LENGTH
            )));
        }
        if let Some(c) = slug
            .chars()
            .find(|c| !(c.is_ascii_lowercase() || c.is_ascii_digit() || *c == '-'))
        {
            return Err(ContributionError::BadSlug(format!(
                "slug contains {c:?}; allowed: a-z 0-9 -"
            )));
        }
        // Counted in characters, not bytes: a title of 120 emoji is 120 to a
        // person and 480 to `len()`, and refusing it would read as a bug.
        let title_chars = title.chars().count();
        if title_chars == 0 {
            return Err(ContributionError::BadTitle("title is empty".into()));
        }
        if title_chars > Self::MAX_TITLE_LENGTH {
            return Err(ContributionError::BadTitle(format!(
                "title exceeds {} characters",
                Self::MAX_TITLE_LENGTH
            )));
        }
        Ok(Self {
            point,
            provider,
            slug: slug.to_owned(),
            title: title.to_owned(),
        })
    }

    #[must_use]
    pub const fn point(&self) -> ExtensionPoint {
        self.point
    }

    #[must_use]
    pub const fn provider(&self) -> &Provider {
        &self.provider
    }

    #[must_use]
    pub fn slug(&self) -> &str {
        &self.slug
    }

    #[must_use]
    pub fn title(&self) -> &str {
        &self.title
    }

    /// The globally unique key: `provider/point/slug`.
    ///
    /// Scoped by provider so two vendors may both contribute a panel called
    /// `summary` without either having to know the other exists — the failure
    /// mode of a flat namespace is that installing a plugin breaks an unrelated
    /// one.
    #[must_use]
    pub fn key(&self) -> String {
        format!("{}/{}/{}", self.provider, self.point.name(), self.slug)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plugin_ids_must_be_reverse_dns() {
        assert!(PluginId::parse("com.example.qa-signoff").is_ok());
        assert_eq!(
            PluginId::parse("qasignoff"),
            Err(PluginIdError::NotReverseDns)
        );
        assert_eq!(PluginId::parse("com..x"), Err(PluginIdError::EmptyLabel));
        assert_eq!(PluginId::parse(".com.x"), Err(PluginIdError::EmptyLabel));
        assert_eq!(PluginId::parse("com.x."), Err(PluginIdError::EmptyLabel));
        assert_eq!(PluginId::parse(""), Err(PluginIdError::Empty));
    }

    #[test]
    fn uppercase_is_refused_rather_than_folded() {
        // Folding would make `com.Example.x` and `com.example.x` the same
        // plugin here and different ones in a case-sensitive store.
        assert_eq!(
            PluginId::parse("com.Example.x"),
            Err(PluginIdError::IllegalCharacter('E'))
        );
    }

    #[test]
    fn an_over_long_id_is_refused_before_it_reaches_an_audit_row() {
        let long = format!("com.{}", "a".repeat(PluginId::MAX_LENGTH));
        assert_eq!(PluginId::parse(&long), Err(PluginIdError::TooLong));
    }

    #[test]
    fn two_providers_may_use_the_same_slug() {
        let core = Contribution::new(ExtensionPoint::UiTaskPanel, Provider::Core, "summary", "S")
            .expect("valid");
        let vendor = Contribution::new(
            ExtensionPoint::UiTaskPanel,
            Provider::Plugin(PluginId::parse("com.example.qa").expect("valid")),
            "summary",
            "S",
        )
        .expect("valid");
        assert_ne!(core.key(), vendor.key());
    }

    #[test]
    fn a_title_is_bounded_in_characters_not_bytes() {
        let emoji = "🙂".repeat(Contribution::MAX_TITLE_LENGTH);
        assert!(
            Contribution::new(ExtensionPoint::UiCommand, Provider::Core, "x", &emoji).is_ok(),
            "120 emoji is 120 characters to a person"
        );
        let one_too_many = "🙂".repeat(Contribution::MAX_TITLE_LENGTH + 1);
        assert!(
            Contribution::new(
                ExtensionPoint::UiCommand,
                Provider::Core,
                "x",
                &one_too_many
            )
            .is_err()
        );
    }

    #[test]
    fn a_slug_that_would_need_escaping_is_refused() {
        for bad in ["Summary", "sum mary", "../etc", "sum/mary", ""] {
            assert!(
                Contribution::new(ExtensionPoint::UiTaskPanel, Provider::Core, bad, "T").is_err(),
                "{bad:?} should be refused"
            );
        }
    }
}
