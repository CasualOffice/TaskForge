# 46 — Observability & Operations

What is instrumented, what is alerted, and what an operator does when something
breaks. Written now rather than after the first incident, because the design
decisions that make a system diagnosable (correlation IDs, outbox lag, per-plugin
metrics) have to be in the architecture, not bolted on.

## The three signals

**Traces** (OpenTelemetry) — one span per request, child spans for authorization,
each database query, each external call. Sampled at 1% baseline, **100% for
errors and slow requests**, and 100% for a named workspace when an operator turns
it on to debug a specific customer.

**Metrics** (Prometheus via Micrometer-equivalent) — RED per endpoint, plus the
domain metrics below.

**Logs** (structured JSON) — one line per request plus explicit events. Every line
carries `request_id`, `correlation_id`, `workspace_id`, `actor_id`, and
`trace_id`.

## Correlation

`correlation_id` is generated at the edge and propagated through the outbox into
every downstream effect ([25](25-EVENTS-OUTBOX-AND-AUDIT.md)):

```
user clicks "Done"
  → request_id R, correlation_id C
  → transition committed          [C]
  → outbox event                  [C]
  → automation matched            [C]
  → automation created a subtask  [C]
  → notification sent             [C]
  → webhook delivered             [C]
```

One query on `C` reconstructs the entire causal chain. Without it, "why did this
task appear?" is unanswerable — and it is the single most common support question
once automations exist.

## Domain metrics

Beyond RED, these are the ones that describe *this* system's health:

| Metric | Why it matters |
| --- | --- |
| `outbox_lag_seconds` — **gauge**, by `consumer` (D-047, settled) | **The primary health signal.** Moves first under database pressure, consumer failure, or a dead worker. It is the age of the oldest **actionable** pending delivery: a single current value, so a gauge — there is only ever one oldest, and a histogram would report percentiles over repeated readings of the same number. "Actionable" excludes deliveries inside their backoff window and deliveries already dead-lettered; counting those would make the primary signal rise during normal retry behaviour and stay high forever after one permanent failure. |
| `outbox_dlq_depth` — gauge, by `consumer` and `event_type` | Deliveries that gave up. Never expected to be non-zero. A dead letter is one `(event, consumer)` pair since migration [0013](../migrations/0013_outbox_delivery.sql), not an event — "which consumer" is the first question RB-02 asks. |
| `outbox_dispatch_total` — counter, by `consumer` and `outcome` | Delivery attempts and how they ended. Answers "is the dispatcher running at all", which the lag gauge cannot distinguish from "it is running and everything is slow". |
| `search_projection_lag_seconds` | How stale search is |
| `authz_resolution_duration` + cache hit ratio | Permission cost, on every request |
| `authz_epoch_bumps_total` | Cache churn; a spike means mass permission change |
| `permission_denied_total` by permission | A burst signals compromise or misconfiguration |
| `transition_rejected_total` by reason | Workflow friction, per project |
| `plugin_call_duration`, `plugin_call_errors_total`, `plugin_call_timeouts_total` by installation | Per-plugin health ([34](34-PLUGIN-AND-EXTENSION-ARCHITECTURE.md)) |
| `plugin_circuit_state` by installation | Which integrations are down |
| `automation_runs_total`, `automation_depth_exceeded_total` | Runaway rules |
| `sse_connections_active` | Live-update load |
| `db_pool_utilization`, `db_query_duration` by statement | Saturation |
| `attachment_scan_queue_depth` | Files stuck invisible |
| `rate_limit_hits_total` by workspace **bucket** — see D-042 | Whether throttling is concentrated. *Not* which tenant: "by workspace" here contradicts §Cardinality discipline below, and that contradiction is open, not settled. |

**Cardinality discipline:** `workspace_id` appears as a raw label on **no** metric
— a 10,000-tenant deployment would produce an unusable time series database.
Per-workspace detail lives in logs and traces, which are queryable by tenant. The
exception is a small allow-list of workspaces under active investigation, enabled
temporarily.

### How that rule is enforced, and with what numbers

A rule survives until the eleventh engineer, so it is a mechanism in
`crates/casual-task-observability` instead. The numbers it picks are recorded
here rather than living only in the source:

| Constant | Value | What it bounds |
| --- | ---: | --- |
| `WORKSPACE_BUCKET_COUNT` | 64 | Tenants are hashed (FNV-1a) into this many buckets. Caps the series count per metric regardless of tenant count. Enough resolution to distinguish "one tenant is causing this" from "everyone is", which is the diagnostic question; it does **not** identify a tenant. |
| `MAX_LABELS_PER_METRIC` | 6 | Series count is the product of label cardinalities, so this is a second, blunter guard. |
| `InvestigationAllowList::MAX_ENTRIES` | 8 | How many tenants may carry a raw id at once — the "small allow-list" above. |

`LabelValue` has no `From<String>`, no `From<Uuid>`, and no `Display`-based
constructor, so a runtime identifier cannot become a label by accident. The two
constructors that *can* widen cardinality are named, cost-documented, and bound
to the one label key each is for.

**"Temporarily" is not yet enforced.** Admission to the allow-list has no
expiry: nothing revokes it, and an admitted tenant produces a per-tenant series
until an operator removes it. That is the one half of this paragraph the code
does not deliver — tracked as **D-042**, with the revocation step in
[50](50-RUNBOOKS.md) until it is.

## Alerts

Alert on **symptoms users feel**, not on causes. High CPU is not an alert; slow
requests are.

| Alert | Condition | Severity |
| --- | --- | --- |
| API error rate | 5xx > 1% for 5 min | page |
| API latency | p95 > 2× target for 10 min | page |
| Outbox lag | p95 > 30 s for 5 min | page |
| DLQ growth | any increase sustained 15 min | page |
| Database pool | > 90% for 5 min | page |
| Auth failure spike | 10× baseline | page (security) |
| Permission-denied spike | 10× baseline for one actor | ticket (security) |
| Search projection lag | > 60 s | ticket |
| Plugin circuits open | > 3 installations in a workspace | ticket |
| Attachment scan queue | > 100 for 15 min | ticket |
| Automation depth exceeded | any | ticket |
| Disk / storage quota | > 85% | ticket |
| Certificate expiry | < 14 days | ticket |

## Runbooks

Each is a document with symptom → diagnosis → action → verification. The set is a
Phase 0 deliverable, not a Phase 4 one:

1. **Outbox lag rising** — is the dispatcher alive? is one consumer slow (check
   per-consumer duration)? is the database saturated? Scale workers; if a single
   consumer is the cause, pause it — the others keep draining.
2. **DLQ growing** — inspect a sample; a payload bug needs a fix and replay, an
   endpoint outage needs the circuit left open until it recovers.
3. **Plugin circuit storm** — identify the installation, notify the workspace
   admin, disable if it is degrading core latency. Core requests should already
   be unaffected; if they are not, that is a defect in failure isolation and a
   post-incident item.
4. **Search stale** — projection worker health, then rebuild from `task` (a
   documented, resumable, throttled operation).
5. **Database failover** — connection draining, replica promotion, verifying the
   outbox did not lose or double-dispatch (at-least-once means duplicates are
   acceptable; loss is not).
6. **Permission incident** — how to use `/permissions/explain` and the audit log
   to establish what an actor could actually do, and when it changed.
7. **Restore from backup** — the drill, run each phase ([15](15-CI-AND-RELEASE-GATES.md)).

## Health endpoints

| Endpoint | Meaning |
| --- | --- |
| `/health/live` | the process is running — never touches the database |
| `/health/ready` | the database is reachable, migrations applied, pool healthy |
| `/health/startup` | slow initial checks, for orchestrator startup probes |

`live` must not check dependencies. A liveness probe that fails during a database
blip restarts every healthy instance simultaneously and converts a partial outage
into a total one.

## SLOs

| SLO | Target | Window |
| --- | --- | --- |
| Availability (core reads/writes) | 99.9% | 30 d |
| API latency within target | 99% of requests | 30 d |
| Outbox lag < 5 s | 99% | 30 d |
| Notification delivery < 60 s | 99% | 30 d |
| Data durability | no confirmed loss | always |

Error budget policy: two consecutive weeks of budget burn stops feature work in
favour of reliability work. Written down in advance, because the argument is
impossible to have credibly during an incident.

## What is not logged

- Passwords, tokens, session IDs, plugin secrets — ever, at any level.
- Task titles, descriptions, comment bodies — customer content does not belong in
  operational logs. IDs are logged; content is not.
- Full request bodies. Structured, allow-listed fields only.
- PII beyond the actor ID. The audit trail is the place for attributable detail,
  with its own access control ([25](25-EVENTS-OUTBOX-AND-AUDIT.md)).

A log scrubber runs as a last-resort filter, but the primary control is that
content is never passed to the logger in the first place.
