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

include!("label_sets.rs");

#[cfg(test)]
#[path = "labels_tests.rs"]
mod tests;
