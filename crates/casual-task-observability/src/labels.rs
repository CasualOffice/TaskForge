//! Metric labels, and the cardinality guard that keeps a raw tenant id out of
//! them.
//!
//! `docs/46-OBSERVABILITY-AND-OPERATIONS.md` §Cardinality discipline:
//!
//! > `workspace_id` appears as a raw label on **no** metric — a 10,000-tenant
//! > deployment would produce an unusable time series database. Per-workspace
//! > detail lives in logs and traces, which are queryable by tenant. The
//! > exception is a small allow-list of workspaces under active investigation,
//! > enabled temporarily.
//!
//! That paragraph is a rule, and `docs/10-PROJECT-GOAL-AND-STANDARDS.md` §3 says
//! a rule survives until the eleventh engineer. So it is a mechanism here
//! instead, in four parts:
//!
//! 1. **No constructor accepts an unbounded value.** [`LabelValue`] can be built
//!    from `&'static str` (bounded by the source text), `bool`, and nothing else
//!    generic. There is deliberately **no** `From<String>`, no `From<Uuid>`, no
//!    `From<WorkspaceId>`, and no `Display`-based constructor — so no runtime
//!    identifier can become a label by accident.
//! 2. **Tenant detail has exactly two doors, both named and cost-documented.**
//!    [`LabelValue::workspace_bucket`] (bounded to
//!    [`WORKSPACE_BUCKET_COUNT`] series, forever) and
//!    [`InvestigationAllowList`] (bounded to
//!    [`InvestigationAllowList::MAX_ENTRIES`] + 1, and meant to be turned off
//!    again).
//! 3. **A metric declares its legal label keys**, and [`LabelSet::with`] rejects
//!    any other key. A dashboard series that nobody declared cannot appear.
//! 4. **A value from door 2 may only go through the key it was minted for.**
//!    Parts 1–3 are all about *keys*; on their own they let a value whose
//!    cardinality was justified for one label be attached to a different one.
//!    A tenant id admitted for `workspace_investigation` was accepted as a
//!    `scope_kind`, and a plugin installation id as a `statement` — the label
//!    that is bounded precisely because statement names are `&'static`. Both
//!    compiled. [`LabelValue::minted_for`] closes that, and
//!    [`CardinalityError::MisplacedValue`] says which key the value belonged to.
//!
//! **The cost, stated plainly** (`docs/10` §4): the bucket label answers "is
//! throttling concentrated in a few tenants or spread across all of them?" It
//! does **not** answer "which tenant?" — that question is answered from logs and
//! traces, which carry `workspace_id`, or by admitting the tenant to the
//! investigation allow-list for as long as the incident lasts.

use std::borrow::Cow;
use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

use casual_task_model::{PluginInstallationId, WorkspaceId};

use crate::metrics::Metric;

/// How many buckets [`LabelValue::workspace_bucket`] hashes tenants into.
///
/// Fixed and small: this is the number that caps the time series count no matter
/// how many tenants exist. 64 is enough resolution to see "one tenant is causing
/// this" versus "everyone is", which is the diagnostic question the bucket is
/// for.
pub const WORKSPACE_BUCKET_COUNT: u16 = 64;

/// The most labels one metric may declare.
///
/// Series count is the product of label cardinalities, so this is a second,
/// blunter guard behind the per-value ones.
pub const MAX_LABELS_PER_METRIC: usize = 6;

/// A metric label key.
///
/// Always `&'static str`: label *names* come from source code, never from
/// request data. See [`keys`] for the declared set.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct LabelKey(&'static str);

impl LabelKey {
    /// Declare a label key. `const` so the [`keys`] module is compile-time data.
    pub const fn new(name: &'static str) -> Self {
        Self(name)
    }

    /// The key as it appears in the exposition format.
    pub const fn as_str(&self) -> &'static str {
        self.0
    }
}

impl fmt::Display for LabelKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.0)
    }
}

/// The declared label keys. A key not in here cannot be constructed anywhere
/// except through [`LabelKey::new`], which only appears in this module.
pub mod keys {
    use super::LabelKey;

    /// Which outbox consumer (`docs/46` runbook 1 — "is one consumer slow?").
    pub const CONSUMER: LabelKey = LabelKey::new("consumer");
    /// The domain event type, e.g. `task.status.changed` (`docs/25`).
    pub const EVENT_TYPE: LabelKey = LabelKey::new("event_type");
    /// Coarse result of an operation: `allowed` / `denied`, `ok` / `error`.
    pub const OUTCOME: LabelKey = LabelKey::new("outcome");
    /// Why something was rejected — a closed set of reason codes, never free text.
    pub const REASON: LabelKey = LabelKey::new("reason");
    /// A `resource.action` permission key from the closed registry (`docs/04`).
    pub const PERMISSION: LabelKey = LabelKey::new("permission");
    /// A plugin installation (`docs/34`). See [`super::LabelValue::plugin_installation`]
    /// for the cardinality cost.
    pub const INSTALLATION: LabelKey = LabelKey::new("installation");
    /// The extension point a plugin call targeted (`docs/34`).
    pub const EXTENSION_POINT: LabelKey = LabelKey::new("extension_point");
    /// Circuit breaker state: `closed` / `open` / `half_open`.
    pub const CIRCUIT_STATE: LabelKey = LabelKey::new("circuit_state");
    /// A named SQL statement, not the SQL text — statement names are `&'static`.
    pub const STATEMENT: LabelKey = LabelKey::new("statement");
    /// Which connection pool: `primary` / `replica`.
    pub const POOL: LabelKey = LabelKey::new("pool");
    /// What a rate limit was keyed on: `workspace` / `actor` / `ip`.
    pub const SCOPE_KIND: LabelKey = LabelKey::new("scope_kind");
    /// HTTP method.
    pub const METHOD: LabelKey = LabelKey::new("method");
    /// The route *template*, e.g. `/v1/tasks/{id}` — never the resolved path,
    /// which would carry ids (`docs/46` §Cardinality discipline).
    pub const ROUTE: LabelKey = LabelKey::new("route");
    /// Response status class: `2xx` / `4xx` / `5xx`.
    pub const STATUS_CLASS: LabelKey = LabelKey::new("status_class");
    /// The hashed tenant bucket. Never the tenant id — see
    /// [`super::LabelValue::workspace_bucket`].
    pub const WORKSPACE_BUCKET: LabelKey = LabelKey::new("workspace_bucket");
    /// A tenant on the temporary investigation allow-list, or `other`.
    /// See [`super::InvestigationAllowList`].
    pub const WORKSPACE_INVESTIGATION: LabelKey = LabelKey::new("workspace_investigation");
}

/// A metric label value.
///
/// The type exists for what it *cannot* do. It has no `From<String>`, no
/// `From<Uuid>`, no `From<WorkspaceId>`, and no constructor that takes
/// `impl Display` — so there is no way to widen a metric to per-tenant
/// cardinality without naming one of the two constructors that say so.
///
/// The bounded path:
///
/// ```
/// use casual_task_observability::labels::{LabelSet, keys};
/// use casual_task_observability::metrics::TRANSITION_REJECTED_TOTAL;
///
/// let labels = LabelSet::for_metric(TRANSITION_REJECTED_TOTAL)
///     .with(keys::REASON, "guard_failed")?;
/// assert_eq!(labels.pairs(), vec![("reason", "guard_failed")]);
/// # Ok::<(), casual_task_observability::CardinalityError>(())
/// ```
///
/// A raw workspace id does not compile — this is the guard from `docs/46`
/// §Cardinality discipline:
///
/// ```compile_fail
/// use casual_task_observability::labels::{LabelSet, keys};
/// use casual_task_observability::metrics::RATE_LIMIT_HITS_TOTAL;
/// use casual_task_model::WorkspaceId;
///
/// let workspace = WorkspaceId::new();
/// // error[E0277]: the trait bound `LabelValue: From<WorkspaceId>` is not satisfied
/// let _ = LabelSet::for_metric(RATE_LIMIT_HITS_TOTAL).with(keys::WORKSPACE_BUCKET, workspace);
/// ```
///
/// Neither does formatting it first, because there is no `From<String>`:
///
/// ```compile_fail
/// use casual_task_observability::labels::{LabelSet, keys};
/// use casual_task_observability::metrics::RATE_LIMIT_HITS_TOTAL;
/// use casual_task_model::WorkspaceId;
///
/// let workspace = WorkspaceId::new();
/// let _ = LabelSet::for_metric(RATE_LIMIT_HITS_TOTAL)
///     .with(keys::WORKSPACE_BUCKET, workspace.to_string());
/// ```
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct LabelValue {
    value: Cow<'static, str>,
    /// The key this value may be attached to, for values whose cardinality is a
    /// deliberate, documented trade.
    ///
    /// Without this the guard was one-sided. A metric declares which *keys* are
    /// legal, and `with` checked exactly that — but a `LabelValue` carried no
    /// memory of what it was minted for, so any value could be attached to any
    /// declared key. That let a raw workspace id through under `scope_kind`,
    /// and a plugin installation id under `statement` — the label
    /// `metrics.rs` documents as bounded *because* statement names are
    /// `&'static`. Both compiled, both were accepted, and both are the
    /// unbounded series `docs/46` §Cardinality discipline forbids.
    ///
    /// `None` for values fixed by the source text, which are safe anywhere.
    minted_for: Option<LabelKey>,
}

impl LabelValue {
    /// The only general constructor: a value fixed by the source text, so its
    /// cardinality is bounded by the number of literals a developer wrote.
    pub const fn from_static(value: &'static str) -> Self {
        Self {
            value: Cow::Borrowed(value),
            minted_for: None,
        }
    }

    /// A value that may only be attached to `key`.
    fn bound_to(value: String, key: LabelKey) -> Self {
        Self {
            value: Cow::Owned(value),
            minted_for: Some(key),
        }
    }

    /// The key this value may be attached to, if it is restricted to one.
    pub fn minted_for(&self) -> Option<LabelKey> {
        self.minted_for
    }

    /// The value as it appears in the exposition format.
    pub fn as_str(&self) -> &str {
        &self.value
    }

    /// Hash a tenant into one of [`WORKSPACE_BUCKET_COUNT`] buckets.
    ///
    /// This is the explicit opt-in from `docs/46` §Cardinality discipline. It
    /// caps the series count at 64 per metric regardless of tenant count, and
    /// the cost is that it identifies a *bucket*, not a tenant: it tells an
    /// operator whether load is concentrated or spread, and nothing more.
    /// Attribution is a log or trace query, which is where `workspace_id` is
    /// allowed to appear.
    pub fn workspace_bucket(workspace: WorkspaceId) -> Self {
        Self::bound_to(
            WorkspaceBucket::of(workspace).to_string(),
            keys::WORKSPACE_BUCKET,
        )
    }

    /// Label a plugin installation (`docs/46` domain metrics: `plugin_call_duration`
    /// and `plugin_circuit_state` are specified *by installation*).
    ///
    /// **Cost:** unlike the workspace bucket, this is unbounded in principle —
    /// cardinality equals the number of installed plugin installations across
    /// the deployment. `docs/46` accepts that trade for per-plugin health
    /// because "which integration is down" is not answerable any other way, but
    /// on a large deployment this is the metric to watch for series blowup, and
    /// the constructor is named so the trade is visible at every call site.
    pub fn plugin_installation(installation: PluginInstallationId) -> Self {
        Self::bound_to(installation.to_string(), keys::INSTALLATION)
    }
}

impl From<&'static str> for LabelValue {
    fn from(value: &'static str) -> Self {
        Self::from_static(value)
    }
}

impl From<bool> for LabelValue {
    fn from(value: bool) -> Self {
        Self::from_static(if value { "true" } else { "false" })
    }
}

impl fmt::Display for LabelValue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.value)
    }
}

/// A tenant hashed into one of [`WORKSPACE_BUCKET_COUNT`] buckets.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct WorkspaceBucket(u16);

impl WorkspaceBucket {
    /// Bucket a tenant.
    ///
    /// FNV-1a over the UUID bytes: stable across processes and restarts (so a
    /// dashboard series does not move under a rolling deploy) and dependency-free.
    /// It is **not** a cryptographic hash and is trivially reversible by brute
    /// force over known tenant ids — which is acceptable, because the bucket is
    /// not a secret, it is a cardinality cap.
    pub fn of(workspace: WorkspaceId) -> Self {
        const FNV_OFFSET_BASIS: u64 = 0xcbf2_9ce4_8422_2325;
        const FNV_PRIME: u64 = 0x0000_0100_0000_01b3;

        let mut hash = FNV_OFFSET_BASIS;
        for byte in workspace.as_uuid().as_bytes() {
            hash ^= u64::from(*byte);
            hash = hash.wrapping_mul(FNV_PRIME);
        }
        Self((hash % u64::from(WORKSPACE_BUCKET_COUNT)) as u16)
    }

    /// The bucket index, in `0..WORKSPACE_BUCKET_COUNT`.
    pub const fn index(&self) -> u16 {
        self.0
    }
}

impl fmt::Display for WorkspaceBucket {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // Zero-padded so buckets sort lexically in a dashboard legend.
        write!(f, "b{:02}", self.0)
    }
}

/// The documented exception in `docs/46` §Cardinality discipline: "a small
/// allow-list of workspaces under active investigation, enabled temporarily."
///
/// Bounded by [`Self::MAX_ENTRIES`], so admitting the whole tenant base during
/// an incident is an error rather than an outage. Everything not admitted labels
/// as [`Self::OTHER`], so the series count is `MAX_ENTRIES + 1`.
///
/// "Temporarily" is not enforced here — this type has no clock. Expiry belongs
/// to whatever configures it, and that is an unimplemented gap, not a solved
/// problem.
#[derive(Debug, Clone, Default)]
pub struct InvestigationAllowList {
    admitted: BTreeSet<WorkspaceId>,
}

impl InvestigationAllowList {
    /// The cap on simultaneously investigated tenants.
    pub const MAX_ENTRIES: usize = 8;

    /// The label every non-admitted tenant collapses into.
    pub const OTHER: LabelValue = LabelValue::from_static("other");

    /// An empty allow-list — the correct steady state.
    pub fn new() -> Self {
        Self::default()
    }

    /// Admit a tenant to per-tenant metric labelling for the duration of an
    /// investigation.
    ///
    /// # Errors
    ///
    /// [`CardinalityError::AllowListFull`] once [`Self::MAX_ENTRIES`] distinct
    /// tenants are admitted. Re-admitting an already-admitted tenant is not an
    /// error and does not consume a slot.
    pub fn admit(&mut self, workspace: WorkspaceId) -> Result<(), CardinalityError> {
        if !self.admitted.contains(&workspace) && self.admitted.len() >= Self::MAX_ENTRIES {
            return Err(CardinalityError::AllowListFull {
                max: Self::MAX_ENTRIES,
            });
        }
        self.admitted.insert(workspace);
        Ok(())
    }

    /// End an investigation.
    pub fn revoke(&mut self, workspace: WorkspaceId) {
        self.admitted.remove(&workspace);
    }

    /// How many tenants are currently admitted.
    pub fn len(&self) -> usize {
        self.admitted.len()
    }

    /// Whether no investigation is active — the steady state.
    pub fn is_empty(&self) -> bool {
        self.admitted.is_empty()
    }

    /// The label for a tenant: its id if admitted, [`Self::OTHER`] otherwise.
    ///
    /// This is the **only** function in the crate that puts a raw workspace id
    /// into a metric label, and it can only do so for a bounded, explicitly
    /// admitted set.
    pub fn label(&self, workspace: WorkspaceId) -> LabelValue {
        if self.admitted.contains(&workspace) {
            LabelValue::bound_to(workspace.to_string(), keys::WORKSPACE_INVESTIGATION)
        } else {
            Self::OTHER
        }
    }
}

/// A label that would have widened a metric past what it declared.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum CardinalityError {
    /// The metric did not declare this label key.
    #[error(
        "metric `{metric}` does not declare label `{key}` (declared: {declared}); \
         see docs/46 §Cardinality discipline"
    )]
    UndeclaredLabel {
        /// The metric the label was attached to.
        metric: &'static str,
        /// The offending key.
        key: &'static str,
        /// The keys the metric does declare, comma separated.
        declared: String,
    },

    /// A cardinality-bearing value was attached to a key it was not minted for.
    #[error(
        "label `{key}` on metric `{metric}` was given a value minted for \
         `{minted_for}`. That value's cardinality is a documented trade for \
         `{minted_for}` only; on `{key}` it is an unbounded series \
         (docs/46 §Cardinality discipline)"
    )]
    MisplacedValue {
        /// The metric the label was attached to.
        metric: &'static str,
        /// The key it was attached to.
        key: &'static str,
        /// The key the value may be used with.
        minted_for: &'static str,
    },

    /// The metric declared more labels than [`MAX_LABELS_PER_METRIC`].
    #[error("metric `{metric}` declares {declared} labels; the cap is {max}")]
    TooManyLabels {
        /// The offending metric.
        metric: &'static str,
        /// How many it declared.
        declared: usize,
        /// The cap.
        max: usize,
    },

    /// [`InvestigationAllowList`] is full.
    #[error(
        "investigation allow-list is full ({max} workspaces); \
         revoke one before admitting another (docs/46 §Cardinality discipline)"
    )]
    AllowListFull {
        /// The cap.
        max: usize,
    },
}

/// The labels attached to one observation of one metric.
///
/// Built against a [`Metric`], which declares the legal keys; an undeclared key
/// is a [`CardinalityError`] rather than a new time series.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LabelSet {
    metric: Metric,
    // BTreeMap: label order must be deterministic, or two observations of the
    // same series render differently in tests and in the exposition output.
    pairs: BTreeMap<LabelKey, LabelValue>,
}

impl LabelSet {
    /// Begin a label set for `metric`.
    pub fn for_metric(metric: Metric) -> Self {
        Self {
            metric,
            pairs: BTreeMap::new(),
        }
    }

    /// Attach a label.
    ///
    /// # Errors
    ///
    /// [`CardinalityError::UndeclaredLabel`] if the metric did not declare this
    /// key; [`CardinalityError::TooManyLabels`] if the metric's own declaration
    /// exceeds [`MAX_LABELS_PER_METRIC`].
    pub fn with(
        mut self,
        key: LabelKey,
        value: impl Into<LabelValue>,
    ) -> Result<Self, CardinalityError> {
        let declared = self.metric.labels();
        if declared.len() > MAX_LABELS_PER_METRIC {
            return Err(CardinalityError::TooManyLabels {
                metric: self.metric.name().as_str(),
                declared: declared.len(),
                max: MAX_LABELS_PER_METRIC,
            });
        }
        if !declared.contains(&key) {
            return Err(CardinalityError::UndeclaredLabel {
                metric: self.metric.name().as_str(),
                key: key.as_str(),
                declared: declared
                    .iter()
                    .map(LabelKey::as_str)
                    .collect::<Vec<_>>()
                    .join(", "),
            });
        }
        let value = value.into();
        // The other half of the guard. Declaring which keys are legal says
        // nothing about what may be *put* in them, and the two cardinality
        // constructors exist precisely because their values are expensive.
        // Attaching one to a different key spends that cost on a metric that
        // never agreed to it.
        if let Some(minted_for) = value.minted_for()
            && minted_for != key
        {
            return Err(CardinalityError::MisplacedValue {
                metric: self.metric.name().as_str(),
                key: key.as_str(),
                minted_for: minted_for.as_str(),
            });
        }
        self.pairs.insert(key, value);
        Ok(self)
    }

    /// The metric these labels belong to.
    pub fn metric(&self) -> Metric {
        self.metric
    }

    /// The labels, in deterministic key order.
    pub fn pairs(&self) -> Vec<(&'static str, &str)> {
        self.pairs
            .iter()
            .map(|(k, v)| (k.as_str(), v.as_str()))
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::metrics::{self, PLUGIN_CIRCUIT_STATE, RATE_LIMIT_HITS_TOTAL};

    #[test]
    fn a_declared_label_is_accepted() {
        let labels = LabelSet::for_metric(metrics::PERMISSION_DENIED_TOTAL)
            .with(keys::PERMISSION, "task.delete")
            .expect("permission is declared on permission_denied_total");
        assert_eq!(labels.pairs(), vec![("permission", "task.delete")]);
    }

    #[test]
    fn a_tenant_bucket_cannot_be_smuggled_onto_another_key() {
        // Both keys are declared on this metric, so the key check passes and
        // only the value check can stop it. Before that check existed, this
        // produced `[("scope_kind", "019fe1ca-…")]` — a raw workspace UUID in a
        // metric label, which is the exact thing docs/46 forbids.
        let workspace = WorkspaceId::new();
        let mut allow = InvestigationAllowList::default();
        allow.admit(workspace).expect("empty list has room");

        let err = LabelSet::for_metric(RATE_LIMIT_HITS_TOTAL)
            .with(keys::SCOPE_KIND, allow.label(workspace))
            .expect_err("an admitted tenant id must not be usable as a scope kind");
        assert!(
            matches!(
                err,
                CardinalityError::MisplacedValue {
                    key: "scope_kind",
                    minted_for: "workspace_investigation",
                    ..
                }
            ),
            "{err}"
        );
    }

    #[test]
    fn an_installation_id_cannot_be_used_as_a_statement_name() {
        // `statement` is documented as bounded *because* statement names are
        // `&'static`. An installation id there is unbounded cardinality on a
        // histogram, which is the most expensive shape there is.
        let err = LabelSet::for_metric(metrics::DB_QUERY_DURATION)
            .with(
                keys::STATEMENT,
                LabelValue::plugin_installation(casual_task_model::PluginInstallationId::new()),
            )
            .expect_err("an installation id is not a statement name");
        assert!(
            matches!(
                err,
                CardinalityError::MisplacedValue {
                    key: "statement",
                    minted_for: "installation",
                    ..
                }
            ),
            "{err}"
        );
    }

    #[test]
    fn a_bounded_value_still_goes_where_it_belongs() {
        // The counterweight: the guard must not break the two legitimate uses.
        let bucket = LabelValue::workspace_bucket(WorkspaceId::new());
        LabelSet::for_metric(RATE_LIMIT_HITS_TOTAL)
            .with(keys::WORKSPACE_BUCKET, bucket)
            .expect("a bucket belongs on workspace_bucket");

        LabelSet::for_metric(PLUGIN_CIRCUIT_STATE)
            .with(
                keys::INSTALLATION,
                LabelValue::plugin_installation(casual_task_model::PluginInstallationId::new()),
            )
            .expect("an installation id belongs on installation");

        // And a source-text value stays usable on any declared key.
        LabelSet::for_metric(metrics::PERMISSION_DENIED_TOTAL)
            .with(keys::PERMISSION, "task.delete")
            .expect("a static value is safe anywhere");
    }

    #[test]
    fn an_undeclared_label_is_rejected() {
        let err = LabelSet::for_metric(metrics::PERMISSION_DENIED_TOTAL)
            .with(keys::STATEMENT, "select_task")
            .expect_err("statement is not declared on permission_denied_total");
        assert!(matches!(
            err,
            CardinalityError::UndeclaredLabel {
                metric: "permission_denied_total",
                key: "statement",
                ..
            }
        ));
    }

    #[test]
    fn no_metric_declares_a_raw_tenant_label() {
        // The guard from docs/46 §Cardinality discipline, asserted over the
        // whole registry so a new metric cannot reintroduce it.
        let banned = [
            "workspace_id",
            "workspace",
            "tenant",
            "tenant_id",
            "actor_id",
        ];
        for metric in metrics::ALL {
            for key in metric.labels() {
                assert!(
                    !banned.contains(&key.as_str()),
                    "metric `{}` declares `{}` — docs/46 forbids raw tenant labels",
                    metric.name(),
                    key
                );
            }
        }
    }

    #[test]
    fn every_metric_stays_under_the_label_cap() {
        for metric in metrics::ALL {
            assert!(
                metric.labels().len() <= MAX_LABELS_PER_METRIC,
                "metric `{}` declares {} labels; cap is {MAX_LABELS_PER_METRIC}",
                metric.name(),
                metric.labels().len()
            );
        }
    }

    #[test]
    fn bucketing_caps_the_series_count_regardless_of_tenant_count() {
        // The whole point: 10,000 tenants must not produce 10,000 series.
        let buckets: BTreeSet<_> = (0..10_000)
            .map(|_| WorkspaceBucket::of(WorkspaceId::new()).index())
            .collect();
        assert!(
            buckets.len() <= usize::from(WORKSPACE_BUCKET_COUNT),
            "bucketing produced {} distinct series",
            buckets.len()
        );
        assert!(
            buckets.iter().all(|b| *b < WORKSPACE_BUCKET_COUNT),
            "a bucket index escaped its range"
        );
        // A degenerate hash that maps everything to one bucket would also pass
        // the cap; assert the spread is real.
        assert!(
            buckets.len() > usize::from(WORKSPACE_BUCKET_COUNT) / 2,
            "bucket distribution is too concentrated to be diagnostic: {} used",
            buckets.len()
        );
    }

    #[test]
    fn bucketing_is_stable_across_calls() {
        // A dashboard series must not move under a rolling deploy.
        let workspace = WorkspaceId::new();
        let first = WorkspaceBucket::of(workspace);
        for _ in 0..100 {
            assert_eq!(WorkspaceBucket::of(workspace), first);
        }
    }

    #[test]
    fn a_bucket_label_never_contains_the_tenant_id() {
        let workspace = WorkspaceId::new();
        let value = LabelValue::workspace_bucket(workspace);
        assert!(
            !value.as_str().contains(&workspace.to_string()),
            "the tenant id leaked into a metric label: {value}"
        );
        let labels = LabelSet::for_metric(RATE_LIMIT_HITS_TOTAL)
            .with(keys::WORKSPACE_BUCKET, value)
            .expect("workspace_bucket is declared on rate_limit_hits_total");
        let rendered = format!("{:?}", labels.pairs());
        assert!(!rendered.contains(&workspace.to_string()));
    }

    #[test]
    fn the_investigation_allow_list_is_bounded() {
        let mut list = InvestigationAllowList::new();
        assert!(list.is_empty(), "the steady state is no investigation");

        let admitted: Vec<_> = (0..InvestigationAllowList::MAX_ENTRIES)
            .map(|_| {
                let workspace = WorkspaceId::new();
                list.admit(workspace).expect("within the cap");
                workspace
            })
            .collect();
        assert_eq!(list.len(), InvestigationAllowList::MAX_ENTRIES);

        // Re-admitting does not consume a slot.
        list.admit(admitted[0]).expect("already admitted");
        assert_eq!(list.len(), InvestigationAllowList::MAX_ENTRIES);

        let err = list
            .admit(WorkspaceId::new())
            .expect_err("the cap is the mechanism");
        assert_eq!(
            err,
            CardinalityError::AllowListFull {
                max: InvestigationAllowList::MAX_ENTRIES
            }
        );

        list.revoke(admitted[0]);
        list.admit(WorkspaceId::new())
            .expect("revoking frees a slot");
    }

    #[test]
    fn a_tenant_off_the_allow_list_collapses_to_other() {
        let mut list = InvestigationAllowList::new();
        let investigated = WorkspaceId::new();
        let ordinary = WorkspaceId::new();
        list.admit(investigated).expect("within the cap");

        assert_eq!(
            list.label(investigated).as_str(),
            investigated.to_string(),
            "an admitted tenant is labelled by id — the documented exception"
        );
        assert_eq!(list.label(ordinary), InvestigationAllowList::OTHER);
        assert!(
            !list
                .label(ordinary)
                .as_str()
                .contains(&ordinary.to_string()),
            "a non-admitted tenant id reached a label"
        );

        list.revoke(investigated);
        assert_eq!(
            list.label(investigated),
            InvestigationAllowList::OTHER,
            "revoking must stop the per-tenant series"
        );
    }

    #[test]
    fn bool_and_static_str_are_the_only_generic_conversions() {
        assert_eq!(LabelValue::from(true).as_str(), "true");
        assert_eq!(LabelValue::from("open").as_str(), "open");
        let labels = LabelSet::for_metric(PLUGIN_CIRCUIT_STATE)
            .with(keys::CIRCUIT_STATE, "open")
            .expect("circuit_state is declared");
        assert_eq!(labels.pairs(), vec![("circuit_state", "open")]);
    }

    #[test]
    fn labels_render_in_deterministic_order() {
        // Two identical series built in opposite insertion order must render
        // identically, or a dashboard sees two series where there is one.
        let forward = LabelSet::for_metric(metrics::HTTP_REQUEST_DURATION_SECONDS)
            .with(keys::METHOD, "GET")
            .and_then(|s| s.with(keys::ROUTE, "/v1/tasks/{id}"))
            .and_then(|s| s.with(keys::STATUS_CLASS, "2xx"))
            .expect("all three are declared on http_request_duration_seconds");
        let reverse = LabelSet::for_metric(metrics::HTTP_REQUEST_DURATION_SECONDS)
            .with(keys::STATUS_CLASS, "2xx")
            .and_then(|s| s.with(keys::ROUTE, "/v1/tasks/{id}"))
            .and_then(|s| s.with(keys::METHOD, "GET"))
            .expect("all three are declared on http_request_duration_seconds");

        assert_eq!(forward.pairs(), reverse.pairs());
        assert_eq!(
            forward.pairs(),
            vec![
                ("method", "GET"),
                ("route", "/v1/tasks/{id}"),
                ("status_class", "2xx"),
            ]
        );
    }
}
