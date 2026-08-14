//! `/api/v1/reports/run` — a filter plus an aggregation (`docs/38`, ADR-027).
//!
//! # The rule this endpoint is built to obey
//!
//! ADR-027: "a report is a saved filter plus an aggregation, over the same
//! closed field set as everything else. No user-defined SQL, no BI query
//! builder, no calculated fields." Analytics is where trackers quietly become
//! slow — a query nobody bounded, over a table nobody indexed for it, run by
//! everyone at 9am — and the defence is that a report cannot express anything
//! the list query cannot.
//!
//! So this handler is the list handler's pipeline with a different tail:
//! `parse_url` → `resolve` → `validate` → compile. The permission predicate,
//! the tenant predicate and the clause emitter are shared, not re-implemented.
//!
//! # Why the filter arrives in URL form
//!
//! `docs/38` sketches the stored report with an AST-shaped `filter`, and
//! `docs/27` insists there is **one AST with two entry points**. Only the URL
//! entry point exists — it is what the address bar, the client and `curl` all
//! speak — so a report carries the same `{"state": "!COMPLETED,CANCELED"}` map
//! a list URL does. Adding a JSON AST parser here would be a second grammar
//! surface to keep in step with the first, which is the thing `docs/27` names
//! as the failure to avoid.
//!
//! # Why the answer names its own scope
//!
//! `docs/38`: "aggregate numbers are not comparable between viewers. A
//! manager's '47 open' and a guest's '12 open' are both right." The permission
//! filter is injected per viewer, so the same report gives different numbers to
//! different people — and a number quoted without its scope is a number that
//! will be argued about. The response carries the count of projects it was
//! computed over so a client can say so.
//!
//! # What is deliberately missing
//!
//! `cycle_time`, `lead_time`, `time_in_state` and `throughput`. All four read
//! state occupancy, which `docs/38` maintains as a projection
//! (`task_state_interval`) that the outbox worker does not build yet. Computing
//! them by replaying `activity_event` per request is exactly the unbounded
//! query the design forbids, so they are absent rather than slow — and the
//! endpoint refuses them by name rather than silently returning counts.

use std::collections::HashMap;

use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use casual_task_model::{ProjectId, TeamId};
use casual_task_persistence::compile::{
    AuthorizedProjectSet, Bucket, BucketField, Dimension, Interval, Reduce, Span, compile_age,
    compile_created_vs_completed, compile_duration, compile_group_count, compile_throughput,
    compile_time_in_state,
};
use casual_task_persistence::{project, report};
use serde::{Deserialize, Serialize};
use time::OffsetDateTime;

use crate::context::Context;
use crate::error::{ApiError, codes};
use crate::middleware::WorkspaceMember;
use crate::server::{AppState, RequestId};
use crate::unit;
use crate::wire::{self, Body};

/// `docs/38` §Report execution limits: 1,000 result groups.
const MAX_GROUPS: u32 = 1_000;

/// The same ceiling the list query resolves its permission filter under.
const MAX_ACCESSIBLE_PROJECTS: u32 = 500;

#[derive(Debug, Deserialize)]
pub struct RunRequest {
    /// The same `field: value` map a list URL carries (`docs/27` §URL form).
    #[serde(default)]
    pub filter: HashMap<String, String>,
    /// What number. Only `count` today — see the module docs.
    #[serde(default = "count")]
    pub measure: String,
    /// Which slice.
    pub group_by: String,
    /// Which time grain, when the report is a series.
    #[serde(default)]
    pub bucket: Option<BucketRequest>,
    #[serde(default)]
    pub limit: Option<u32>,
    /// Which state `time_in_state` measures. Ignored by every other measure.
    ///
    /// Its own field rather than a suffix on the measure name
    /// (`time_in_state_active`) because the set of states is data — five today,
    /// and `docs/23` owns it — while measure names are a closed vocabulary this
    /// module parses. Folding one into the other would mean every new state
    /// silently becoming a new measure name nobody registered.
    #[serde(default)]
    pub state: Option<String>,
}

fn count() -> String {
    "count".to_owned()
}

#[derive(Debug, Deserialize)]
pub struct BucketRequest {
    pub field: String,
    pub interval: String,
}

#[derive(Debug, Serialize)]
pub struct GroupView {
    /// `null` is a real answer — unassigned, untriaged, on no environment.
    pub key: Option<String>,
    /// Absent unless the report asked for a series.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bucket_start: Option<String>,
    pub total: i64,
}

/// `POST /api/v1/reports/run` — run one, without saving it.
///
/// # Errors
///
/// `400` for an unknown dimension, measure, bucket or an unparseable filter;
/// `500` on a database failure.
pub async fn run(
    State(state): State<AppState>,
    member: WorkspaceMember,
    headers: HeaderMap,
    Body(body): Body<RunRequest>,
) -> Result<Response, ApiError> {
    let request_id = RequestId::of_parts(&headers);
    let dimension = dimension_of(&body.group_by, &request_id)?;
    let measure = measure_of(&body.measure, &request_id)?;
    // Validated before anything is read: an unknown state would compile into a
    // cast that raises at the database, which reaches the caller as a 500 for
    // something they typed. The five are `docs/23`'s permanent states.
    let measured_state = state_for(measure, body.state.as_deref(), &request_id)?;
    let bucket = body
        .bucket
        .as_ref()
        .map(|b| bucket_of(b, &request_id))
        .transpose()?;
    let limit = body.limit.unwrap_or(MAX_GROUPS).clamp(1, MAX_GROUPS);

    let pairs: Vec<(&str, &str)> = body
        .filter
        .iter()
        .map(|(name, value)| (name.as_str(), value.as_str()))
        .collect();
    let query = casual_task_search::parse_url(pairs).map_err(|error| {
        ApiError::bad_request(
            crate::error::Code::from_registry(error.code()),
            "The report's filter could not be understood",
            &request_id,
        )
        .with_details(serde_json::json!({ "field": error.field() }))
    })?;

    let mut tx = unit::begin(&state, &request_id).await?;
    let mut scoped = unit::scope(&mut tx, &member, &request_id).await?;
    let ctx = Context::load(&state.metrics, &mut scoped, &member, &headers, &request_id).await?;

    // `docs/04` §The list problem, step 1 — resolved once, and it *is* the
    // permission filter. A report that resolved it differently from the list
    // would be a second answer to "what may this person see".
    let accessible = project::accessible(&mut scoped, &ctx.viewer, MAX_ACCESSIBLE_PROJECTS)
        .await
        .map_err(|error| {
            tracing::error!(%error, "resolving the accessible project set failed");
            ApiError::internal(&request_id)
        })?;
    let visible: Vec<ProjectId> = accessible
        .iter()
        .map(|(id, _)| ProjectId::from_uuid(*id))
        .collect();
    let scope_size = visible.len();

    let resolver = casual_task_search::Context::new(
        ctx.actor,
        ctx.viewer
            .teams
            .iter()
            .copied()
            .map(TeamId::from_uuid)
            .collect(),
        OffsetDateTime::now_utc(),
        wire::caller_offset(&headers),
    );
    let filter = casual_task_search::resolve(&query.filter, &resolver).map_err(|error| {
        ApiError::bad_request(
            codes::UNKNOWN_SYMBOL,
            "The report's filter uses a symbol this server does not know",
            &request_id,
        )
        .with_details(serde_json::json!({ "symbol": format!("{error:?}") }))
    })?;
    casual_task_search::validate(&filter).map_err(|error| {
        ApiError::bad_request(
            crate::error::Code::from_registry(error.code()),
            "The report's filter exceeds a documented limit",
            &request_id,
        )
    })?;

    let authorized = AuthorizedProjectSet::resolved(visible);
    let compiled = match measure {
        Measure::Count => compile_group_count(
            &filter,
            ctx.workspace,
            &authorized,
            dimension,
            bucket,
            limit,
        ),
        Measure::Duration(span, reduce) => compile_duration(
            &filter,
            ctx.workspace,
            &authorized,
            dimension,
            span,
            reduce,
            limit,
        ),
        // Weekly unless asked otherwise, like throughput: "raised against
        // finished" with no period is two numbers, not two trends.
        Measure::CreatedVsCompleted => compile_created_vs_completed(
            &filter,
            ctx.workspace,
            &authorized,
            bucket.map_or(Interval::Week, |b| b.interval),
            limit,
        ),
        Measure::TimeInState(reduce) => compile_time_in_state(
            &filter,
            ctx.workspace,
            &authorized,
            dimension,
            measured_state.as_str(),
            reduce,
            limit,
        ),
        Measure::Age(reduce) => compile_age(
            &filter,
            ctx.workspace,
            &authorized,
            dimension,
            reduce,
            limit,
        ),
        // Throughput is always a series: "how much finished" without a period
        // is a number nobody can act on. Weekly unless asked otherwise.
        Measure::Throughput => compile_throughput(
            &filter,
            ctx.workspace,
            &authorized,
            dimension,
            bucket.map_or(Interval::Week, |b| b.interval),
            limit,
        ),
    };
    let rows = report::run(&mut scoped, &compiled).await.map_err(|error| {
        tracing::error!(%error, "running the report failed");
        ApiError::internal(&request_id)
    })?;
    unit::commit(tx, &request_id).await?;

    let total: i64 = rows.iter().map(|row| row.total).sum();
    Ok((
        StatusCode::OK,
        axum::Json(serde_json::json!({
            // Echoed as what the answer is *actually* grouped by. For
            // `created_vs_completed` the two series are the grouping — the
            // rows come back keyed `created` and `completed` — so repeating
            // the caller's `group_by` here would describe a slice the answer
            // does not contain. The request field stays required because every
            // other measure needs it; this says plainly that it was not used.
            "group_by": match measure {
                Measure::CreatedVsCompleted => "series",
                _ => body.group_by.as_str(),
            },
            "measure": body.measure,
            // Seconds for a duration, tasks for a count. The client cannot
            // format a number whose unit it has to guess.
            "unit": match measure {
                // Age is a duration too, and a client that formatted it as a
                // task count would render "1209600" where "14.0d" belongs.
                Measure::Duration(_, _) | Measure::Age(_) | Measure::TimeInState(_) => "seconds",
                _ => "tasks",
            },
            "groups": rows
                .into_iter()
                .map(|row| GroupView {
                    key: row.key,
                    bucket_start: row.bucket_start.map(wire::timestamp),
                    total: row.total,
                })
                .collect::<Vec<_>>(),
            "total": total,
            // `docs/38`: a number without its scope is a number that will be
            // argued about. Two people running this see different totals, and
            // both are right.
            "scope": { "projects": scope_size },
        })),
    )
        .into_response())
}

fn dimension_of(name: &str, request_id: &str) -> Result<Dimension, ApiError> {
    match name {
        "status" => Ok(Dimension::Status),
        "state" => Ok(Dimension::State),
        "type" => Ok(Dimension::Type),
        "priority" => Ok(Dimension::Priority),
        "project" => Ok(Dimension::Project),
        "team" => Ok(Dimension::Team),
        "environment" => Ok(Dimension::Environment),
        "reporter" => Ok(Dimension::Reporter),
        "milestone" => Ok(Dimension::Milestone),
        "assignee" => Ok(Dimension::Assignee),
        _ => Err(ApiError::bad_request(
            codes::OUT_OF_RANGE,
            "group_by must be one of status, state, type, priority, project, \
             team, environment, reporter, milestone, assignee",
            request_id,
        )
        .with_details(serde_json::json!({ "group_by": name }))),
    }
}

/// `count` is the measure set this build maintains a bounded query for.
///
/// The others are refused **by name** rather than ignored: a caller who asks
/// for `p50 cycle_time` and silently receives counts has a number that is
/// wrong in a way nothing on the page reveals.
/// What the report is measuring.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Measure {
    Count,
    /// A span reduced across the tasks in each group.
    Duration(Span, Reduce),
    /// How long *open* work has been waiting — `created_at` → now.
    Age(Reduce),
    /// Two series per bucket: raised, and finished.
    CreatedVsCompleted,
    /// How long tasks spent in one permanent state.
    TimeInState(Reduce),
    Throughput,
}

/// `docs/23`'s permanent states — the closed set `time_in_state` may name.
const STATES: &[&str] = &["BACKLOG", "PLANNED", "ACTIVE", "COMPLETED", "CANCELED"];

/// The state a `time_in_state` report measures.
///
/// Required for that measure and refused for every other: a `state` sent
/// alongside `count` would be a parameter the answer ignores, and a caller who
/// believes it narrowed the report has a number that means something else.
fn state_for(measure: Measure, state: Option<&str>, request_id: &str) -> Result<String, ApiError> {
    match (measure, state) {
        (Measure::TimeInState(_), Some(named)) if STATES.contains(&named) => Ok(named.to_owned()),
        (Measure::TimeInState(_), Some(named)) => Err(ApiError::bad_request(
            codes::INVALID_ENUM,
            "state must be one of BACKLOG, PLANNED, ACTIVE, COMPLETED or CANCELED",
            request_id,
        )
        .with_details(serde_json::json!({ "state": named }))),
        (Measure::TimeInState(_), None) => Err(ApiError::bad_request(
            codes::MISSING_FIELD,
            "time_in_state needs `state`: how long in which state",
            request_id,
        )),
        (_, Some(_)) => Err(ApiError::bad_request(
            codes::OUT_OF_RANGE,
            "`state` applies to time_in_state alone; every other measure would ignore it",
            request_id,
        )),
        (_, None) => Ok(String::new()),
    }
}

/// The measure set this build maintains a bounded query for — now all of
/// `docs/38`'s closed set except the two-period comparisons it does not define.
///
/// A measure outside it is refused **by name** rather than ignored: a caller
/// who asks for one and silently receives counts has a number wrong in a way
/// nothing on the page reveals.
fn measure_of(name: &str, request_id: &str) -> Result<Measure, ApiError> {
    match name {
        "count" => Ok(Measure::Count),
        "cycle_time" | "p50_cycle_time" => Ok(Measure::Duration(Span::CycleTime, Reduce::P50)),
        "p90_cycle_time" => Ok(Measure::Duration(Span::CycleTime, Reduce::P90)),
        "avg_cycle_time" => Ok(Measure::Duration(Span::CycleTime, Reduce::Avg)),
        "lead_time" | "p50_lead_time" => Ok(Measure::Duration(Span::LeadTime, Reduce::P50)),
        "p90_lead_time" => Ok(Measure::Duration(Span::LeadTime, Reduce::P90)),
        "avg_lead_time" => Ok(Measure::Duration(Span::LeadTime, Reduce::Avg)),
        // `age` defaults to the **oldest**, not the median. "How old is the
        // work" is usually asked as "what has been sitting longest", and a
        // median hides exactly the one task the question is about. The
        // percentiles are there for anyone who wants the distribution.
        "age" | "max_age" => Ok(Measure::Age(Reduce::Max)),
        "p50_age" => Ok(Measure::Age(Reduce::P50)),
        "p90_age" => Ok(Measure::Age(Reduce::P90)),
        "avg_age" => Ok(Measure::Age(Reduce::Avg)),
        "created_vs_completed" => Ok(Measure::CreatedVsCompleted),
        "throughput" => Ok(Measure::Throughput),
        "time_in_state" | "p50_time_in_state" => Ok(Measure::TimeInState(Reduce::P50)),
        "p90_time_in_state" => Ok(Measure::TimeInState(Reduce::P90)),
        "avg_time_in_state" => Ok(Measure::TimeInState(Reduce::Avg)),
        "max_time_in_state" => Ok(Measure::TimeInState(Reduce::Max)),
        _ => Err(ApiError::bad_request(
            codes::OUT_OF_RANGE,
            "measure must be count, cycle_time, lead_time, age, throughput or \
             created_vs_completed or time_in_state (each duration also as avg_, \
             p50_ or p90_; age and time_in_state also as max_)",
            request_id,
        )
        .with_details(serde_json::json!({ "measure": name }))),
    }
}

fn bucket_of(request: &BucketRequest, request_id: &str) -> Result<Bucket, ApiError> {
    let field = match request.field.as_str() {
        "created_at" => BucketField::CreatedAt,
        "updated_at" => BucketField::UpdatedAt,
        "due_at" => BucketField::DueAt,
        other => {
            return Err(ApiError::bad_request(
                codes::OUT_OF_RANGE,
                "bucket.field must be created_at, updated_at or due_at",
                request_id,
            )
            .with_details(serde_json::json!({ "field": other })));
        }
    };
    let interval = match request.interval.as_str() {
        "day" => Interval::Day,
        "week" => Interval::Week,
        "month" => Interval::Month,
        other => {
            return Err(ApiError::bad_request(
                codes::OUT_OF_RANGE,
                "bucket.interval must be day, week or month",
                request_id,
            )
            .with_details(serde_json::json!({ "interval": other })));
        }
    };
    Ok(Bucket { field, interval })
}

#[cfg(test)]
#[path = "reports_tests.rs"]
mod tests;
