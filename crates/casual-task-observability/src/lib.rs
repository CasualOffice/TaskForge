//! # casual-task-observability
//!
//! Tracing, metrics, and correlation (`docs/46-OBSERVABILITY-AND-OPERATIONS.md`).
//!
//! **Owns:** the tracing subscriber, the metric-name registry, the metric-label
//! cardinality guard, and correlation-id propagation — the thread that ties a
//! user action to every effect it caused.
//!
//! **Must never own:** customer content. Task titles, descriptions, and comment
//! bodies never reach the logger; IDs do. [`Redacted`] exists so that a value
//! which *is* customer content cannot be logged by accident.
//!
//! Boundary contract: `docs/19-WORKSPACE-SCAFFOLD-DESIGN.md`. An illegal
//! dependency here is a build failure, not a review comment.
//!
//! ## The two mechanisms in this crate
//!
//! Both exist because `docs/10-PROJECT-GOAL-AND-STANDARDS.md` §3 requires a
//! mechanism rather than a rule — "a rule survives until the eleventh engineer;
//! a compile error survives."
//!
//! 1. **Cardinality guard** ([`labels`]) — `docs/46` §Cardinality discipline says
//!    `workspace_id` appears as a raw label on **no** metric, because a
//!    10,000-tenant deployment produces an unusable time series database.
//!    [`LabelValue`] therefore has no constructor that accepts a
//!    [`WorkspaceId`](casual_task_model::WorkspaceId), a `Uuid`, or a runtime
//!    `String`. Tenant-adjacent detail is available only through two named,
//!    greppable, cost-documented constructors.
//!
//! 2. **Redaction guard** ([`redact`]) — [`Redacted<T>`] prints `<redacted>` from
//!    `Debug`, `Display`, and `Serialize`, so a task title routed into a log
//!    field is visibly wrong in the output rather than silently exfiltrated.
//!
//! ## What this crate does *not* do yet
//!
//! - **No `/metrics` endpoint.** [`Recorder`] records values and renders the
//!   Prometheus exposition body ([`recorder`]); serving it over HTTP belongs to
//!   `casual-task-api`, because `docs/19` puts every HTTP type there and
//!   `casual-task-lint` enforces it. The endpoint arrives with C-001.
//! - **No OpenTelemetry.** `docs/46` §The three signals specifies OTLP traces and
//!   `docs/48` defines `TF_OTEL_ENDPOINT`; no `opentelemetry` crate is a
//!   workspace dependency, so log lines carry `request_id` and `correlation_id`
//!   but not `trace_id`.
//! - **No log scrubber.** `docs/46` §What is not logged calls the scrubber a
//!   last-resort filter behind the primary control; the primary control
//!   ([`Redacted`]) is what is implemented here.

pub mod correlation;
pub mod labels;
pub mod metrics;
pub mod recorder;
pub mod redact;
pub mod subscriber;

pub use correlation::{CorrelationContext, CorrelationId, RequestId};
pub use labels::{
    CardinalityError, InvestigationAllowList, LabelKey, LabelSet, LabelValue, WorkspaceBucket,
};
pub use metrics::{Metric, MetricKind, MetricName};
pub use recorder::{BUCKETS, Recorder, WrongKind};
pub use redact::{Redact, Redacted};
pub use subscriber::{LOG_FORMAT_ENV, LogFormat, TelemetryConfig, TelemetryError, init, init_with};
