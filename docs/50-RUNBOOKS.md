# 50 — Operational Runbooks

The seven runbooks named in [46](46-OBSERVABILITY-AND-OPERATIONS.md) §Runbooks,
written out. This document is the Phase 0 deliverable **F-014**
([14](14-EXECUTION-TRACKER.md)) and it is a contract with the operator, not a
narrative: each runbook is symptom → diagnosis → action → verification →
prevention, and every diagnostic step carries the command or query that produces
the answer.

Written before the first incident, because a runbook authored during one is
written by the person with the least time and the most adrenaline.

## The set

| # | Runbook | Fired by | Severity | First move |
| --- | --- | --- | --- | --- |
| [RB-01](#rb-01--outbox-lag-rising) | Outbox lag rising | `outbox_lag_seconds` p95 > 30 s / 5 min | page | Measure whether the backlog is growing or draining |
| [RB-02](#rb-02--dead-letter-queue-growing) | Dead-letter queue growing | `outbox_dlq_depth` increase sustained 15 min | page | Classify the errors; **do not replay yet** |
| [RB-03](#rb-03--plugin-circuit-storm) | Plugin circuit storm | `plugin_circuit_state` open > 3 installations in a workspace | ticket | Check whether core latency moved |
| [RB-04](#rb-04--search-projection-stale) | Search projection stale | `search_projection_lag_seconds` > 60 s | ticket | Check outbox lag first — search is downstream |
| [RB-05](#rb-05--database-failover) | Database failover | 5xx > 1% / 5 min, pool > 90%, or a provider notice | page | Confirm which node is primary |
| [RB-06](#rb-06--permission-incident) | Permission incident | Permission-denied spike for one actor, or a customer question | ticket (security) | Separate "can they now" from "could they then" |
| [RB-07](#rb-07--restore-from-backup) | Restore from backup | Confirmed loss, or the scheduled drill | page / planned | Restore to a **scratch** instance, never in place |

Alert definitions and severities are owned by [46](46-OBSERVABILITY-AND-OPERATIONS.md)
§Alerts; latency and lag targets by [30](30-PERFORMANCE-AND-CAPACITY-TARGETS.md);
backup RPO/RTO by [48](48-DEPLOYMENT-PROFILES.md). This document does not restate
them, it links to them ([16](16-DOCUMENTATION-MAINTENANCE.md) rule 1).

## Executability today — read this before following anything

**Phase 0. There is no application code.** The Cargo workspace, the twelve
migrations, the non-superuser application role, and the schema verification gate
exist and are `Gated` (F-015). `casual-task-worker` is a scaffold whose `main`
prints a line; there is no dispatcher, no projection consumer, no metrics
pipeline, no admin console.

Every step below is therefore marked:

| Mark | Meaning |
| --- | --- |
| **✅ executable** | Runs today against a migrated PostgreSQL 16. The SQL was written against the real schema in `migrations/`. |
| **⏳ designed** | The step is correct and final as a design, and cannot be performed yet because the code it names does not exist. |

A runbook step marked ⏳ is not a gap in this document — it is a gap in the
system, recorded here so it is visible rather than discovered at 03:00. The
summary is in [§Executability matrix](#executability-matrix).

## Working on this database

Read this once. It changes what the queries below return.

| Purpose | Connect as | Why |
| --- | --- | --- |
| Cross-tenant diagnostics | a superuser or `BYPASSRLS` admin role | Migration [0010](../migrations/0010_row_level_security.sql) applies `FORCE ROW LEVEL SECURITY`, which enforces policies **for the table owner too**. A non-superuser owner sees zero rows on tenant tables with no scope set. |
| Single-tenant diagnostics | any role, inside `BEGIN; SET LOCAL taskforge.workspace_id = '<uuid>'; … COMMIT;` | The policy compares `workspace_id` to that setting. Unset means `NULL` means no rows — it fails closed. |
| The request path | `taskforge_app` (`NOSUPERUSER NOBYPASSRLS`, migration [0012](../migrations/0012_application_role.sql)) | Never used for operator surgery. It deliberately cannot `UPDATE`/`DELETE` history. |

Three consequences that catch people:

- **`outbox_event` is exempt from RLS** — a documented exemption in migration
  0010, because the dispatcher polls across all tenants. Every query in RB-01 and
  RB-02 runs with no scope set. That exemption is why they work.
- **A query returning zero rows may mean "wrong role", not "no data".** Check
  `SELECT current_user, current_setting('is_superuser');` before concluding
  anything from an empty result.
- **`activity_event` and `audit_event` are partitioned by month, but only the
  default partitions exist today.** Migration 0007 creates
  `activity_event_default` and `audit_event_default`; the retention worker that
  creates monthly partitions ahead of time is ⏳. Queries against the parent
  tables are correct either way; partition pruning gives nothing yet, so
  bound them by `occurred_at` for cost, not for correctness. Creating a monthly
  partition **after** rows for that month have landed in the default partition
  fails — PostgreSQL revalidates the default partition and rejects rows that
  would now belong elsewhere:

  ```
  ERROR:  updated partition constraint for default partition "audit_event_default"
          would be violated by some row
  ```

  Those rows must be moved out of the default partition first. Creating *next*
  month's partition before its rows arrive succeeds, which is why the retention
  worker creates partitions ahead of time rather than on demand
  ([25](25-EVENTS-OUTBOX-AND-AUDIT.md) §Retention and partitioning).

Operator sessions should set their own bound rather than inherit the
application's: `SET LOCAL statement_timeout = '30s';` The 5 s application limit
([21](21-API-LIMITS-AND-QUOTAS.md)) exists to protect users from a runaway
request, not to protect an operator from a deliberate one.

## Six standing rules

These decide the cases where runbooks usually fail.

1. **"Do nothing and wait" is an action, and often the correct one.** A draining
   backlog, a circuit that will close in 60 s, and a managed failover in progress
   are all conditions where intervention makes the outage longer. Each runbook
   names its wait branch explicitly.
2. **Duplicates are acceptable; loss is not.** Delivery is at-least-once
   ([25](25-EVENTS-OUTBOX-AND-AUDIT.md)); consumers are idempotent on `event_id`
   by contract. Deleting outbox rows, or marking them `dispatched_at = now()`, to
   suppress duplicates converts the acceptable failure into the one the design
   forbids. There is no incident in which that is the right fix.
3. **Fix the cause before the replay.** A replay into an unfixed consumer burns
   six more attempts per event and re-runs the work on every healthy consumer as
   well.
4. **History is evidence.** `activity_event` and `audit_event` are append-only by
   grant, not by convention. If an incident seems to call for editing them,
   escalate instead.
5. **A failing `/health/ready` is not a reason to restart anything.**
   `/health/live` deliberately does not touch the database precisely so that a
   database blip does not restart every healthy instance at once
   ([46](46-OBSERVABILITY-AND-OPERATIONS.md) §Health endpoints).
6. **Revoke an investigation admission when the incident closes.**
   [46](46-OBSERVABILITY-AND-OPERATIONS.md) §Cardinality discipline allows a
   tenant onto a per-workspace metric label "temporarily", and **nothing expires
   it** — there is no clock in `InvestigationAllowList` (tracker **D-042**). An
   admission left in place produces a per-tenant series forever, which is the
   cardinality blowup the allow-list exists to prevent. Until the expiry is
   designed, the last step of any incident that admitted a workspace is to
   remove it, and the allow-list holds at most 8, so a forgotten one also
   silently costs the next investigation a slot.

---

## RB-01 — Outbox lag rising

### Symptom

Page: **Outbox lag — `outbox_lag_seconds` p95 > 30 s for 5 minutes.** The SLO is
p95 < 1 s ([30](30-PERFORMANCE-AND-CAPACITY-TARGETS.md) §Throughput) with an
error-budget target of < 5 s at 99% ([46](46-OBSERVABILITY-AND-OPERATIONS.md)
§SLOs).

What users report, if anything: boards stop updating live, notifications arrive
late, newly created tasks are not findable by the search box (that last one is
RB-04's symptom arriving through this cause).

Outbox lag is the **primary health signal**. It moves first under database
pressure, a dead worker, or one slow consumer — so a page here is frequently a
symptom of something else, and the diagnosis order below is arranged to find that
out before touching the dispatcher.

### Diagnosis

**1. Is there a backlog, and how old is it?** ✅ executable

```sql
SELECT d.consumer,
       count(*)                                                 AS pending,
       count(*) FILTER (WHERE d.next_attempt_at <= now())       AS actionable,
       now() - min(e.created_at) FILTER (WHERE d.next_attempt_at <= now())
                                                                AS oldest_actionable,
       count(*) FILTER (WHERE d.attempts > 0)                   AS retrying,
       count(*) FILTER (WHERE d.dead_lettered_at IS NOT NULL)   AS dead_lettered
  FROM outbox_delivery d
  JOIN outbox_event e ON e.id = d.event_id
 WHERE d.dispatched_at IS NULL
 GROUP BY d.consumer
 ORDER BY oldest_actionable DESC NULLS LAST;
```

**Read `oldest_actionable`, not `oldest_pending`.** They differ, and the
difference is the whole point: a delivery inside its backoff window is *waiting
on purpose*. Counting it as lag makes the primary health signal rise during
normal retry behaviour, which is how a paging alert gets muted (D-047).

The breakdown is **per consumer** because delivery state is per consumer
(migration [0013](../migrations/0013_outbox_delivery.sql)). One row per consumer
with a large `pending` and five healthy ones is the single most common shape of
this incident, and it used to be invisible here.

`dead_lettered_at IS NOT NULL` is the dead-letter condition — not `attempts >=
6`. The ladder in [25](25-EVENTS-OUTBOX-AND-AUDIT.md) is six attempts, but the
state is now recorded explicitly rather than inferred from a count, so a change
to the ladder length cannot silently change what this query means. If
`dead_lettered` dominates, this is **RB-02**, not RB-01: dead rows are never
dispatched and never leave the table on their own.

**2. Is it growing or draining?** ✅ executable

```sql
SELECT date_trunc('minute', e.created_at)                       AS minute,
       count(*)                                                AS created,
       count(*) FILTER (WHERE d.dispatched_at IS NOT NULL)     AS dispatched,
       count(*) FILTER (WHERE d.dispatched_at IS NULL)         AS still_pending
  FROM outbox_delivery d
  JOIN outbox_event e ON e.id = d.event_id
 WHERE e.created_at > now() - interval '30 minutes'
 GROUP BY 1
 ORDER BY 1;
```

The window is complete: dispatched rows are deleted 7 days after dispatch, not
sooner. If `still_pending` falls minute over minute and `oldest_actionable`
from step 1 shrinks between two samples 60 s apart, **the system is recovering on its
own** — go to the wait branch in Action.

**3. Is the dispatcher alive?** ⏳ designed

Check `outbox_dispatch_batch_total` for a rising counter and the worker's
`/health/ready`. On the single-node profile there is no separate worker: the
dispatcher runs embedded in the API process (`TF_WORKER_EMBEDDED=true`,
[48](48-DEPLOYMENT-PROFILES.md)), so "is the worker up" is "is the API up".

The database-side approximation, available now: ✅ executable

```sql
SELECT pid, usename, state, wait_event_type, wait_event,
       now() - query_start AS running_for, left(query, 120) AS query
  FROM pg_stat_activity
 WHERE query ILIKE '%outbox_delivery%'
   AND state <> 'idle'
 ORDER BY query_start;
```

Seeing no dispatcher poll at all over several samples means no dispatcher.
Reading other users' `query` text requires `pg_monitor` or superuser; as an
unprivileged role the column is `<insufficient privilege>`.

**4. Which events are failing, and with what?** ✅ executable

```sql
SELECT d.consumer,
       e.event_type,
       count(*)                        AS pending,
       max(d.attempts)                 AS worst_attempts,
       left(min(d.last_error), 200)    AS sample_error
  FROM outbox_delivery d
  JOIN outbox_event e ON e.id = d.event_id
 WHERE d.dispatched_at IS NULL
   AND d.attempts > 0
 GROUP BY d.consumer, e.event_type
 ORDER BY pending DESC
 LIMIT 20;
```

This answers "which consumer is failing, and on what" directly. It did not used
to: `last_error` was a single column on the event, holding the most recent
failure across the whole fan-out, so a webhook returning 502 and a search
projection timing out overwrote each other and the operator had to infer the
culprit from event types. Per-consumer delivery state (migration
[0013](../migrations/0013_outbox_delivery.sql)) is what makes the error
attributable.

Corroborate with `plugin_call_duration` and the per-consumer metrics in
[46](46-OBSERVABILITY-AND-OPERATIONS.md) (⏳) when they exist — a consumer that
is *slow* rather than *failing* has few errors here and a rising lag in step 1.

**5. Is the database the actual cause?** ✅ executable

```sql
SELECT count(*)                                          AS connections,
       count(*) FILTER (WHERE state = 'active')          AS active,
       count(*) FILTER (WHERE wait_event_type = 'Lock')  AS waiting_on_locks
  FROM pg_stat_activity
 WHERE datname = current_database();

SELECT pid, now() - xact_start AS xact_age, state, left(query, 100) AS query
  FROM pg_stat_activity
 WHERE xact_start < now() - interval '1 minute'
 ORDER BY xact_start;
```

A long-running transaction blocks the dispatcher's `FOR UPDATE SKIP LOCKED` from
making progress on the rows it holds, and pins vacuum. If `db_pool_utilization`
is also alerting, the outbox is a symptom — fix the saturation.

**6. Is the pending index doing its job?** ✅ executable

```sql
SELECT relname, n_live_tup, n_dead_tup, last_autovacuum, last_autoanalyze
  FROM pg_stat_user_tables
 WHERE relname IN ('outbox_event', 'outbox_delivery');

EXPLAIN (ANALYZE, BUFFERS)
SELECT id FROM outbox_delivery
 WHERE dispatched_at IS NULL AND dead_lettered_at IS NULL
   AND next_attempt_at <= now()
 ORDER BY next_attempt_at, created_at
 LIMIT 100;
```

`outbox_delivery` is the high-churn table now, and it churns **six times harder
than the event table** — one row per consumer per event, each updated at claim
and again at record. The plan must use `outbox_delivery_pending_ix`. A sequential
scan here, or a dead-tuple count far above the live count, means autovacuum is
not keeping up and the poll itself has become the bottleneck.

### Action

| Diagnosis | Action | Cost |
| --- | --- | --- |
| Backlog draining, `oldest_pending` shrinking (step 2) | **Do nothing.** Re-check in 15 minutes. Silence the alert for the drain window rather than acting on it. | Continued lag for the drain duration. A bulk operation, an import, or a deploy legitimately produces a spike; intervening adds a variable to a system that is already recovering. |
| No dispatcher running (step 3) | Restart the worker (or the API on the single-node profile). Undispatched rows are picked up on the next poll; rows a dead worker held under `FOR UPDATE` were released when its connection dropped. | Some events dispatch twice. Acceptable (rule 2). |
| One consumer failing, others fine (step 4) | Nothing, first. Per-consumer delivery state means a failing consumer already backs off on its own ladder without touching the other five — confirm that in step 1 before acting. To stop it entirely: ⏳ there is no consumer registry to pause against yet. | Delivery to that consumer stops until it is resumed; its deliveries stay pending, and after six attempts land in the DLQ (RB-02). The other five are unaffected — which is what migration 0013 bought. |
| Database saturated or blocked (step 5) | Fix the database first: kill the blocking transaction, relieve the pool. Then re-measure. | — |
| Vacuum/bloat (step 6) | `VACUUM (ANALYZE) outbox_delivery;` as the owner, and lower `autovacuum_vacuum_scale_factor` for the table. | A manual vacuum competes for I/O with the workload it is meant to relieve. Run it, then watch. |
| Sustained genuine volume, dispatcher healthy, database healthy | Scale workers. Beyond ~10,000 events/s, shard dispatch by `workspace_id` hash ([48](48-DEPLOYMENT-PROFILES.md) Profile 3). ⏳ | More workers means more connections. On Profile 1 there is nothing to scale — the ceiling is the ceiling. |

**Do not:**

- **Do not clear the backlog by marking rows dispatched.** `UPDATE outbox_delivery
  SET dispatched_at = now() WHERE dispatched_at IS NULL` makes the alert stop and
  silently discards every undelivered event — no SSE update, no notification, no
  search projection, no webhook. This is the exact failure the outbox exists to
  prevent ([25](25-EVENTS-OUTBOX-AND-AUDIT.md) §Why the outbox exists from day
  one). There is no incident where it is correct.
- **Do not add workers while the pool is above 90%.** More pollers contending for
  a saturated pool lengthens the queue.
- **Do not restart the API "to clear it."** Restarting is safe (in-flight rows are
  redelivered) but it addresses nothing, and on Profile 1 it drops every SSE
  stream at the same time.

### Verification

- `oldest_actionable` from step 1 below 5 s across three consecutive samples,
  **for every consumer** — an aggregate that looks healthy can hide one starved
  consumer.
- `outbox_lag_seconds` p95 back under 1 s ⏳ (the metric is part of F-009).
- `pending` returned to its pre-incident baseline — not zero. A healthy outbox is
  never empty on a busy system; it is *young*.
- The `EXPLAIN` in step 6 uses `outbox_delivery_pending_ix`.

### Prevention

- Outbox lag is already a paging alert on a symptom rather than a cause
  ([46](46-OBSERVABILITY-AND-OPERATIONS.md) §Alerts).
- Worker pools separated by role on Profile 3, so a webhook backlog cannot starve
  dispatch ([48](48-DEPLOYMENT-PROFILES.md)).
- Bulk operations capped at 100 tasks per request with an async job path above
  that ([05](05-API-SPEC.md) §Bulk operations) — the main source of self-inflicted
  spikes.
- The at-least-once acceptance test (kill the dispatcher mid-batch; assert every
  event is delivered, some twice, none lost) is a named gate in
  [25](25-EVENTS-OUTBOX-AND-AUDIT.md) §Acceptance gates. ⏳
- **Closed** (was: "open design item, surfaced not decided"). This runbook
  warned that dead-lettered rows are the *oldest* pending rows, so a poll
  selecting on `dispatched_at IS NULL` alone would re-read them on every pass and
  a growing DLQ would degrade dispatch latency for healthy events. Migration
  [0013](../migrations/0013_outbox_delivery.sql) removes the possibility rather
  than documenting the care needed: `outbox_delivery_pending_ix` is partial on
  `dispatched_at IS NULL AND dead_lettered_at IS NULL`, so dead rows leave the
  index the moment they are dead-lettered. The claim query cannot see them
  whatever it asks for.

---

## RB-02 — Dead-letter queue growing

### Symptom

Page: **DLQ growth — any increase sustained for 15 minutes.** `outbox_dlq_depth`
is never expected to be non-zero
([46](46-OBSERVABILITY-AND-OPERATIONS.md) §Domain metrics), so this alert fires on
*movement*, not on a threshold.

Definition, concretely: `outbox_delivery.dead_lettered_at IS NOT NULL` — set
after six attempts across the backoff ladder 1 s, 4 s, 16 s, 1 m, 5 m, 30 m
([25](25-EVENTS-OUTBOX-AND-AUDIT.md) §Retry and dead-letter), indexed by
`outbox_delivery_dlq_ix`.

**A dead letter is one (event, consumer) pair, not an event.** The same event can
be dead for the webhook consumer and delivered fine to the other five, and the
counts below are of deliveries. An operator who reads them as events will
overestimate the blast radius by up to six times.

**A dead-lettered event is not lost.** It is durable, indexed, and replayable. The
urgency is that it is *undelivered*, and that it will stay that way forever: the
7-day cleanup only removes rows that were dispatched.

### Diagnosis

**1. Shape and age.** ✅ executable

```sql
SELECT d.consumer,
       e.event_type,
       count(*)               AS dead,
       min(e.created_at)      AS oldest,
       max(e.created_at)      AS newest
  FROM outbox_delivery d
  JOIN outbox_event e ON e.id = d.event_id
 WHERE d.dead_lettered_at IS NOT NULL
 GROUP BY d.consumer, e.event_type
 ORDER BY dead DESC;
```

**2. One tenant or many?** ✅ executable

```sql
SELECT workspace_id,
       count(*)                      AS dead,
       count(DISTINCT consumer)      AS consumers_affected,
       left(min(last_error), 200)    AS sample_error
  FROM outbox_delivery
 WHERE dead_lettered_at IS NOT NULL
 GROUP BY workspace_id
 ORDER BY dead DESC
 LIMIT 20;
```

One workspace means a customer-side endpoint or a single bad installation. Many
workspaces at once means our code, our egress, or our deploy. `consumers_affected`
sharpens it further: one consumer across many workspaces is that consumer's
problem, all six in one workspace is that workspace's data. That distinction
decides the entire action branch, so run this before anything else.

(`workspace_id` in a query result is fine. It must never become a metric label —
[46](46-OBSERVABILITY-AND-OPERATIONS.md) §Cardinality discipline.)

**3. Group the errors.** ✅ executable

```sql
SELECT consumer,
       left(last_error, 120)   AS error_class,
       count(*)                AS n,
       min(created_at)         AS first_seen,
       max(dead_lettered_at)   AS last_seen
  FROM outbox_delivery
 WHERE dead_lettered_at IS NOT NULL
 GROUP BY 1, 2
 ORDER BY n DESC
 LIMIT 20;
```

**4. Inspect a sample — metadata only.** ✅ executable

```sql
SELECT d.id AS delivery_id, d.consumer, d.attempts, d.last_error,
       d.dead_lettered_at,
       e.id AS event_id, e.event_type, e.aggregate_type, e.aggregate_id,
       e.workspace_id, e.schema_version, e.created_at
  FROM outbox_delivery d
  JOIN outbox_event e ON e.id = d.event_id
 WHERE d.dead_lettered_at IS NOT NULL
 ORDER BY e.created_at
 LIMIT 5;
```

`e.payload` is deliberately not selected. Event payloads carry changed field values
— which for `task.created` includes the title
([25](25-EVENTS-OUTBOX-AND-AUDIT.md) §Event envelope). Customer content does not
belong in an operator's terminal scrollback or in a ticket. If a payload must be
read to diagnose a schema bug, read one row, in a session you will not paste
from, and quote the *shape*, never the values.

**5. Classify.** The error text from step 3 puts the incident in exactly one of
these:

| Class | Looks like | Cause sits with |
| --- | --- | --- |
| Endpoint outage | connection refused, timeout, 502/503 | The customer's service |
| Contract / payload bug | 400, 422, deserialization error, `schema_version` mismatch | Us, or a plugin pinned to an old contract |
| Authorization | 401, 403, token expired, installation revoked | Consent or credential state |
| Poison event | panic, unwrap, index out of range in a consumer | Us |

### Action

| Class | Action |
| --- | --- |
| Endpoint outage, customer-run | **Do nothing to the events.** Leave the circuit open — it will close on its own when the endpoint recovers ([34](34-PLUGIN-AND-EXTENSION-ARCHITECTURE.md)). Notify the workspace admin through the plugin health view. Replay after the endpoint is confirmed healthy, not before. Waiting here is the whole action. |
| Contract / payload bug | Fix the code. Deploy. **Then** replay, scoped to the affected consumer. Replaying first burns six more attempts and re-runs delivery for nothing. |
| Authorization | Do not replay. An uninstalled or revoked installation has no valid destination — the uninstall lifecycle drops its queued jobs by design ([34](34-PLUGIN-AND-EXTENSION-ARCHITECTURE.md) §Uninstall). If a token merely expired, rotate it, then replay. |
| Poison event | Fix the consumer to reject the event at the contract boundary rather than panic ([34](34-PLUGIN-AND-EXTENSION-ARCHITECTURE.md) §Failure isolation). A consumer that can be killed by one malformed row will be. |

**Replay.** ⏳ designed — entries are replayable from the admin console after the
cause is fixed ([25](25-EVENTS-OUTBOX-AND-AUDIT.md)). The console is the right
surface because a replay is an operator action that should itself be audited.

The database-level equivalent, for the period before that console exists —
bounded, never global:

```sql
-- Run as the owner/migration role, inside a transaction, after the cause is fixed.
-- ALWAYS bounded by consumer and a time window. Never the whole table.
BEGIN;
UPDATE outbox_delivery d
   SET dead_lettered_at = NULL,
       attempts         = 0,
       next_attempt_at  = now(),
       last_error       = NULL
  FROM outbox_event e
 WHERE e.id = d.event_id
   AND d.dead_lettered_at IS NOT NULL
   AND d.consumer = 'webhook_delivery'
   AND e.created_at >= '2026-08-08T09:00:00Z'
   AND e.created_at <  '2026-08-08T10:00:00Z';
-- Read the row count. If it is not what step 1 predicted, ROLLBACK.
COMMIT;
```

**Replay one consumer, not the event.** This is the difference migration
[0013](../migrations/0013_outbox_delivery.sql) makes to this procedure: a replay
used to reset the event and re-deliver to all six consumers, so fixing a broken
webhook meant re-running the search projection and re-sending every notification
for those events. Users saw duplicate notifications for a webhook incident they
had no part in. Scoping the `UPDATE` to `d.consumer` re-delivers only to the one
that failed — and omitting that predicate is now the mistake to watch for, since
the query still runs and still looks correct.

Then re-run the step 1 query and watch the DLQ fall. Replay in windows, not all
at once: a large replay is indistinguishable from the traffic spike in RB-01.

**Do not:**

- **Do not delete DLQ rows to clear the alert.** "A dead-lettered event is never
  silently dropped" is the contract ([25](25-EVENTS-OUTBOX-AND-AUDIT.md)).
  Deleting is the one irreversible action available here.
- **Do not replay before the cause is fixed** (standing rule 3).
- **Do not treat redelivery as free.** It is *safe* — consumers are idempotent on
  `event_id` by contract — but the consumer pays the work again. Leaving the
  `d.consumer` predicate off the replay above spreads that cost to all six,
  including the notification fan-out, which is the one users notice.
- **Do not edit `payload` to "fix" a bad event.** The payload is what the
  transaction committed. Rewriting it makes the event stream disagree with the
  history that was written alongside it in the same transaction.

### Verification

- The step 1 query returns zero rows, or returns only the classes deliberately
  left in place (a revoked installation's events, pending an admin decision) with
  a note in the incident record saying so.
- No new dead-letters for 30 minutes after the replay — long enough to cover the
  full backoff ladder plus margin.
- The consumer's own metric (`plugin_call_errors`, notification delivery rate) is
  back at baseline ⏳.

### Prevention

- Circuit breakers per installation, so a dead endpoint stops being called rather
  than dead-lettering everything ([34](34-PLUGIN-AND-EXTENSION-ARCHITECTURE.md)).
- Event schema versioning with both versions delivered during a deprecation
  window, so a pinned consumer does not start rejecting payloads on our deploy
  day ([25](25-EVENTS-OUTBOX-AND-AUDIT.md)).
- Event-schema-diff CI gate: a payload change requires a `schema_version` bump
  ([15](15-CI-AND-RELEASE-GATES.md) §Contracts).
- The DLQ acceptance test — a consumer failing six times dead-letters, alerts, and
  is replayable ([25](25-EVENTS-OUTBOX-AND-AUDIT.md) §Acceptance gates). ⏳

---

## RB-03 — Plugin circuit storm

### Symptom

Ticket: **Plugin circuits open — more than 3 installations open in one
workspace.** Deliberately a ticket, not a page: no plugin can fail a core request
([34](34-PLUGIN-AND-EXTENSION-ARCHITECTURE.md) §Failure isolation), so an open
circuit is the system working, not the system failing.

Supporting signals: `plugin.circuit.opened` audit events, `plugin.timeout` and
`plugin.validation.skipped` audit events, workspace admins reporting panels that
render an inline error.

**Escalate to a page the moment core latency moves.** If `task read` or `status
transition` p95 leaves its target in [30](30-PERFORMANCE-AND-CAPACITY-TARGETS.md)
while circuits are open, failure isolation has not held — that is a defect in the
core, and the plugin is the trigger rather than the fault.

### Diagnosis

⏳ throughout: plugins ship in Phase 3
([34](34-PLUGIN-AND-EXTENSION-ARCHITECTURE.md) §Delivery) and the audit writer in
Phase 1. The tables exist today and the queries below run — they return zero rows
until then.

**1. Which installations, and how concentrated?**

```sql
-- Cross-tenant: run as a superuser/BYPASSRLS role. audit_event carries
-- workspace_id, so RLS applies (migration 0010).
SELECT workspace_id,
       target_id            AS installation_id,
       count(*)             AS opens,
       max(occurred_at)     AS last_open
  FROM audit_event
 WHERE event_type = 'plugin.circuit.opened'
   AND occurred_at > now() - interval '1 hour'
 GROUP BY 1, 2
 ORDER BY opens DESC;
```

**2. Is it one plugin across many tenants, or many plugins in one tenant?**

```sql
SELECT pi.plugin_id,
       count(DISTINCT pi.workspace_id)                       AS workspaces,
       count(*)                                              AS installations,
       count(*) FILTER (WHERE pi.enabled)                    AS enabled
  FROM plugin_installation pi
 WHERE pi.uninstalled_at IS NULL
   AND pi.id IN (SELECT target_id FROM audit_event
                  WHERE event_type = 'plugin.circuit.opened'
                    AND occurred_at > now() - interval '1 hour')
 GROUP BY pi.plugin_id
 ORDER BY installations DESC;
```

One plugin, many workspaces → the plugin vendor's service is down; every
customer of it is affected and none of them caused it. Many unrelated plugins,
many workspaces, starting at the same minute → suspect **our** egress: DNS, the
outbound proxy, the allow-list, or a deploy. Check what changed on our side
before contacting anyone.

**3. Did core latency move?**

Compare `task read`, `task update`, and `status transition` p95 against
[30](30-PERFORMANCE-AND-CAPACITY-TARGETS.md). ⏳ — this needs the metrics
pipeline. Every plugin interaction is outside the core transaction and the only
synchronous point is `validation.transition`, bounded at 500 ms
([34](34-PLUGIN-AND-EXTENSION-ARCHITECTURE.md)), so a correct implementation
cannot move core latency by more than that bound per transition.

**4. Is a fail-closed validator involved?**

`validation.transition` fails **open** by default (ADR-017), and an admin may opt
a specific plugin into fail-closed knowingly. Fail-closed plus an outage means
the team cannot move work — that is the branch that turns this ticket into a
customer-impacting incident.

```sql
SELECT id, workspace_id, plugin_id, version, enabled, granted_scopes, config
  FROM plugin_installation
 WHERE id = $1;
```

Read `config` for the per-installation `on_failure` override. Read it; do not
edit it.

```sql
SELECT date_trunc('minute', occurred_at) AS minute,
       event_type,
       count(*)                          AS n
  FROM audit_event
 WHERE workspace_id = $1
   AND event_type IN ('plugin.validation.skipped', 'plugin.timeout')
   AND occurred_at > now() - interval '30 minutes'
 GROUP BY 1, 2
 ORDER BY 1;
```

A steady stream of `plugin.validation.skipped` is fail-open doing its job:
transitions are being allowed. Their **absence** while transitions are being
rejected is the fail-closed case.

### Action

| Diagnosis | Action |
| --- | --- |
| One installation, fail-open, core latency normal | **Do nothing to the installation.** Notify the workspace admin through the per-installation health view ([34](34-PLUGIN-AND-EXTENSION-ARCHITECTURE.md) §Observability). The circuit closes on its own 60 s after the endpoint recovers. Disabling a customer's integration to quiet a self-healing ticket is a customer-visible action with no operational benefit. |
| One installation, fail-closed, transitions blocked | Contact the workspace admin. **They** flip it to fail-open. An operator overriding it unilaterally reverses a deliberate compliance choice; do it without the admin only under an explicit incident authority, and record the decision as an audit-visible action. |
| Many plugins, many workspaces, simultaneous | Treat as our outage. Check egress, DNS, proxy, allow-list, and the last deploy. Roll back the deploy rather than disabling other people's plugins. |
| Core latency degraded | Disable the installation as load shedding (`enabled = false` via the admin path), **and** open a defect. "No plugin can fail a core request" is non-negotiable; if one did, the isolation is broken and that is the post-incident item ([34](34-PLUGIN-AND-EXTENSION-ARCHITECTURE.md), and it is named as a chaos-test gap in [15](15-CI-AND-RELEASE-GATES.md) §Future gates). |
| Quota exhaustion rather than failure | The plugin is throttled and the admin is notified by design; core is unaffected. No operator action. |

**Do not:**

- **Do not uninstall to stop the noise.** Uninstall is a lifecycle, not a delete:
  it destroys tokens and starts a 30-day grace period on plugin-owned data
  ([34](34-PLUGIN-AND-EXTENSION-ARCHITECTURE.md) §Uninstall). Disable is
  reversible; uninstall is not free.
- **Do not raise the 500 ms timeout** to stop the circuit tripping. The bound is
  what keeps a slow plugin from becoming a slow product.
- **Do not hand-edit `granted_scopes`, `manifest_hash`, or `secret_ref`.** Those
  three columns record what an admin consented to, and they are the evidence in
  any later question about what the plugin was permitted to do.
- **Do not restart the API to reset circuits.** Circuit state is per installation
  and time-bounded; a restart resets it into the same failing endpoint and
  restarts the storm.

### Verification

- No `plugin.circuit.opened` audit events for 15 minutes.
- `plugin_circuit_state` closed for the affected installations ⏳.
- Core p95 within the [30](30-PERFORMANCE-AND-CAPACITY-TARGETS.md) targets.
- If a fail-closed validator was involved: a test transition in the affected
  project succeeds.

### Prevention

- Fail-open by default, with fail-closed as an explicit, warned opt-in
  (ADR-017) — the single decision that keeps a vendor outage from stopping work.
- Per-installation circuit breakers, quotas, and egress allow-lists
  ([34](34-PLUGIN-AND-EXTENSION-ARCHITECTURE.md)).
- Plugin health visible to the workspace admin, not only to operators — so the
  customer sees their own integration is the cause without opening a ticket.
- **Chaos test: plugin storm** is a named future gate
  ([15](15-CI-AND-RELEASE-GATES.md) §Future gates). Until it exists, the claim
  that core is unaffected is a design property that has not been measured. Stated
  plainly rather than assumed.

---

## RB-04 — Search projection stale

### Symptom

Ticket: **Search projection lag > 60 s.** Target is p95 < 2 s
([30](30-PERFORMANCE-AND-CAPACITY-TARGETS.md) §Throughput).

What users report: "I created a task two minutes ago and search cannot find it" —
while the same task appears correctly in the board, the project list, and any
filtered view.

**That split is designed behaviour, not a bug.** Structured filters read `task`
directly and are strictly consistent; full-text reads the `task_search`
projection and is eventually consistent, typically under a second
([26](26-SEARCH-INDEXING-AND-QUERY.md) §The search projection). A few seconds of
staleness is the system operating as specified. This runbook is about *minutes*.

### Diagnosis

**1. Confirm the outbox is not the cause — check it first.** ✅ executable

```sql
SELECT count(*)                    AS pending,
       now() - min(e.created_at)   AS oldest_pending
  FROM outbox_delivery d
  JOIN outbox_event e ON e.id = d.event_id
 WHERE d.consumer = 'search_projection'
   AND d.dispatched_at IS NULL
   AND d.dead_lettered_at IS NULL;
```

Scoped to `search_projection`, because that is the only consumer that can make
the index stale. A global outbox count would implicate a webhook backlog that has
nothing to do with search.

The projection is maintained by an outbox consumer. If the outbox is backed up,
**this is RB-01** and there is nothing to do here. Fixing the dispatcher fixes
search.

**2. Measure the staleness directly, bounded.** ✅ executable

```sql
-- Bounded by updated_at so it rides task_updated_brin instead of scanning task.
-- Run as superuser/BYPASSRLS for a cross-tenant view, or scoped per tenant.
SELECT count(*) FILTER (WHERE s.task_id IS NULL)             AS never_indexed,
       count(*) FILTER (WHERE s.updated_at < t.updated_at)   AS stale,
       now() - min(t.updated_at)                             AS oldest_affected
  FROM task t
  LEFT JOIN task_search s ON s.task_id = t.id
 WHERE t.deleted_at IS NULL
   AND t.updated_at > now() - interval '1 hour';
```

**Cost, stated:** the unbounded form of this query — dropping the `updated_at`
predicate — is a full join across `task` and `task_search`. At the reference
corpus of 2M tasks that is minutes of I/O on the primary during an incident.
Widen the window in steps (1 hour → 6 hours → 1 day) only if the bounded form
shows nothing, and prefer scoping to one workspace.

**3. One scope or global?** ✅ executable

```sql
SELECT t.workspace_id, t.project_id, count(*) AS stale
  FROM task t
  LEFT JOIN task_search s ON s.task_id = t.id
 WHERE t.deleted_at IS NULL
   AND t.updated_at > now() - interval '1 hour'
   AND (s.task_id IS NULL OR s.updated_at < t.updated_at)
 GROUP BY 1, 2
 ORDER BY stale DESC
 LIMIT 20;
```

Concentration in one project is a bulk import or a bulk transition
([05](05-API-SPEC.md) §Bulk operations). Spread evenly is a consumer problem.

**4. Are the documents wrong rather than missing?** ⏳ designed

A weighting change, a `tsvector` configuration change, or a migration touching
the document construction produces rows that are present, current by
`updated_at`, and wrong. Nothing in the schema detects this — the check is
querying for a term you know is in a task and confirming it ranks. Cluster the
affected rows around a deploy time to confirm.

**5. Is the consumer alive?** ⏳ designed — `search_projection_lag_seconds` and
the consumer's health come from F-009 and `casual-task-search`, neither of which
exists.

### Action

| Diagnosis | Action | Cost |
| --- | --- | --- |
| Outbox also backed up | Go to RB-01. **Do nothing here.** | — |
| Draining after a bulk import, `stale` falling | **Do nothing.** Re-check in 15 minutes. A projection backlog after an import is expected, which is exactly why this alert is a ticket and not a page. | Search stays behind for the drain window. Tell the reporting user to use a filter, which is strictly consistent. |
| Consumer dead or wedged | Restart it. Undispatched events are redelivered; projection writes are an upsert keyed on `task_id`, so redelivery converges. ⏳ | Duplicate work, no duplicate rows. |
| A bounded set of stale rows after a fixed bug | Targeted rebuild for the affected workspace/project only. ⏳ | Proportional to the scope, not to the corpus. |
| Documents wrong after a config change | Full rebuild — throttled, resumable, off-peak. ⏳ | Hours at reference scale. See below. |

**Rebuild.** ⏳ designed — a documented, resumable, throttled operation
([46](46-OBSERVABILITY-AND-OPERATIONS.md) §Runbooks item 4). The batch shape,
fixed here so the tool and this runbook agree:

```sql
-- Resumable because it orders by primary key: after an interruption, restart
-- from the last completed id. $2 is that id (use the nil UUID to start).
SELECT id
  FROM task
 WHERE deleted_at IS NULL
   AND workspace_id = $1
   AND id > $2
 ORDER BY id
 LIMIT 1000;
```

Costs, stated plainly:

- The search document spans `task`, `comment`, `task_tag`/`tag`, and assignee and
  reporter display names ([26](26-SEARCH-INDEXING-AND-QUERY.md) §Weighting), so
  each batch reads several tables. A rebuild is not a single-table scan.
- At the 2M-task reference corpus, budget **hours**, not minutes. Throttle it and
  run it off-peak; an unthrottled rebuild competes with the live workload for the
  same GIN indexes it is writing.
- The rebuild upserts in place. Search stays available, returning older documents
  for rows not yet reached.

**Do not:**

- **Do not `TRUNCATE task_search` and rebuild.** Search returns nothing for the
  entire rebuild — hours of total search outage to fix seconds of staleness.
  Upsert in place.
- **Do not add a GIN index to `task` so search is "always current."**
  [26](26-SEARCH-INDEXING-AND-QUERY.md) rejects this explicitly: GIN maintenance
  on the hot write path with bursty pending-list flushes produces exactly the
  latency spike a drag-and-drop board must not have.
- **Do not rebuild while the outbox is backed up.** The rebuild competes with the
  dispatcher for the database and the consumer that would drain the backlog.
- **Do not describe this to users as "search is broken."** The filter path is
  fine and is the workaround.

### Verification

- The step 2 query returns `never_indexed = 0` and `stale = 0` for the affected
  scope.
- A task created now is findable by full text within 2 s.
- `search_projection_lag_seconds` back under 2 s ⏳.

### Prevention

- Projection lag alerted separately from outbox lag, so a stale search is not
  mistaken for a healthy one ([46](46-OBSERVABILITY-AND-OPERATIONS.md)).
- Worker pools separated by role on Profile 3, so a webhook backlog cannot delay
  the projection ([48](48-DEPLOYMENT-PROFILES.md)).
- The projection is a separate table precisely so it can be rebuilt without
  touching the write path ([26](26-SEARCH-INDEXING-AND-QUERY.md)).
- The rebuild tool is drilled once per phase alongside the restore drill. A
  rebuild path that has never been run is the same class of hypothesis as an
  unrestored backup.

---

## RB-05 — Database failover

### Symptom

Page, usually more than one at once: **API error rate 5xx > 1% for 5 minutes**,
**Database pool > 90%**, and/or **Outbox lag** — plus, on a managed provider, a
failover notification.

Health endpoints during a failover:

| Endpoint | Expected | Meaning |
| --- | --- | --- |
| `/health/live` | **passing** | The process is fine. It never touches the database, by design. |
| `/health/ready` | failing | The database is unreachable; the instance should leave the load balancer rotation. |
| `/health/startup` | n/a | Startup probes only. |

If `/health/live` is failing during a database blip, that is a **configuration
defect**, not a symptom of the failover — and it is about to restart every
healthy instance simultaneously, converting a partial outage into a total one
([46](46-OBSERVABILITY-AND-OPERATIONS.md) §Health endpoints).

### Diagnosis

**1. Which node am I on?** ✅ executable

```sql
SELECT pg_is_in_recovery()      AS is_replica,
       inet_server_addr()       AS server,
       current_setting('server_version') AS version;
```

`true` means the connection landed on a replica; writes fail with `cannot execute
INSERT in a read-only transaction`.

**2. Replication state.** ✅ executable

On the primary:

```sql
SELECT client_addr, state, sent_lsn, write_lsn, flush_lsn, replay_lsn,
       write_lag, flush_lag, replay_lag
  FROM pg_stat_replication;
```

On a replica:

```sql
SELECT now() - pg_last_xact_replay_timestamp() AS replay_delay,
       pg_last_wal_receive_lsn(),
       pg_last_wal_replay_lsn();
```

**3. Is the application connecting as the right role?** ✅ executable — run this
before declaring the incident over.

```sql
SELECT current_user,
       current_setting('is_superuser')                AS is_superuser,
       (SELECT rolbypassrls FROM pg_roles
         WHERE rolname = current_user)                AS bypasses_rls;
```

If `is_superuser` is `on`, **stop and escalate**: row-level security and the
append-only revoke are both inert for a superuser
([0012](../migrations/0012_application_role.sql)). A failover onto a differently
provisioned node, or an emergency DSN edit, is exactly how an availability
incident silently becomes a tenancy incident. Migration 0012 documents a startup
check that refuses to boot in this state; until that code exists ⏳, this query is
the check.

**4. What was in flight?** ✅ executable

```sql
SELECT d.consumer,
       count(*)          AS pending,
       min(e.created_at) AS oldest,
       max(e.created_at) AS newest
  FROM outbox_delivery d
  JOIN outbox_event e ON e.id = d.event_id
 WHERE d.dispatched_at IS NULL
   AND d.dead_lettered_at IS NULL
 GROUP BY d.consumer
 ORDER BY oldest;
```

**5. Where did the data actually stop?** ✅ executable

```sql
SELECT max(occurred_at) AS last_audit  FROM audit_event;
SELECT max(created_at)  AS last_outbox FROM outbox_event;
SELECT max(updated_at)  AS last_task   FROM task;   -- needs a tenant scope or BYPASSRLS
```

Compare against the last pre-failover monitoring sample. A gap equal to the
replica lag at promotion time is expected on an asynchronous replica and is the
RPO being spent ([48](48-DEPLOYMENT-PROFILES.md) §Backups and disaster recovery).

### Action

| Diagnosis | Action |
| --- | --- |
| Provider failover in progress; `live` passing, `ready` failing | **Do nothing to the application.** Let the pools reconnect. Restarting instances during a failover turns a 30-second reconnect into a cold-start stampede against a database that is still coming up. |
| `/health/live` failing on a database blip | Fix the probe — `live` must not check dependencies. **Do not roll the fleet.** Restarting healthy instances is the failure this endpoint design exists to prevent. |
| Promotion complete, app still pointed at the old primary | Repoint: failover DNS, the pooler, or `DATABASE_URL`. Drain and restart only the instances wedged on stale connections, one at a time. |
| Old primary returns | Fence it before anything else. Two writable primaries against `task` is not recoverable by any procedure in this document. |
| App is connecting as a superuser (step 3) | Repoint to the `taskforge_app` DSN immediately and treat it as a tenancy incident: RLS was off for the duration. |
| Pool saturated after recovery from reconnect storms | Lower the pool ceiling temporarily and let it refill, rather than raising it into a database that is still catching up. |

**The outbox after a failover — read this before touching a single row.**

Delivery is at-least-once ([25](25-EVENTS-OUTBOX-AND-AUDIT.md) §Delivery
semantics). Three post-failover observations, and what each means:

| Observation | Meaning | Action |
| --- | --- | --- |
| A consumer received an event twice | **Expected.** A dispatcher delivered, then lost its connection before setting `dispatched_at`. The row was redelivered after promotion. | None. Consumers are idempotent on `event_id` by contract. |
| A duplicate produced a visible side effect (two webhooks, two emails) | That consumer is not idempotent. | A defect in the consumer, not in the outbox. Fix the consumer. **Do not delete rows.** |
| A committed domain change has no outbox row | **Loss.** Either an event was written outside the transaction, or the promotion replayed past a commit boundary. | Escalate. This is the failure the whole design forbids ([25](25-EVENTS-OUTBOX-AND-AUDIT.md)), and it is a correctness incident, not an availability one. |

**Do not delete outbox rows, or set `dispatched_at`, to suppress duplicates after
a failover.** Duplicates are acceptable and were planned for; loss is not, and
every such "cleanup" converts the acceptable failure into the unacceptable one,
irreversibly. There is no version of this incident in which suppressing
duplicates is worth the risk of suppressing a real event.

Transactions that did not commit left nothing behind — the atomic write means a
change and its three history rows commit together or not at all
([25](25-EVENTS-OUTBOX-AND-AUDIT.md) §The atomic write). The outbox never
contains a phantom event for a change that did not happen.

### Verification

- `pg_is_in_recovery()` returns `false` on the write DSN.
- `/health/ready` green on every instance; error rate under 1% for 15 minutes.
- Step 3 returns `is_superuser = off` and `bypasses_rls = f`.
- Tenant isolation and append-only still hold. The assertions in
  [`scripts/verify-schema.sh`](../scripts/verify-schema.sh) are the canonical
  form: an unscoped session sees zero rows; a scoped session sees only its
  tenant; `UPDATE`/`DELETE` on `audit_event` as `taskforge_app` fails.
- Pending outbox drains to its pre-incident baseline (RB-01 verification).
- The RPO actually spent is recorded against the profile's target
  ([48](48-DEPLOYMENT-PROFILES.md)) — measured, not assumed.

### Prevention

- WAL archiving and PITR on Profile 2 and above; the single-node profile's 24 h
  RPO is a stated, accepted cost ([48](48-DEPLOYMENT-PROFILES.md)).
- Failover at the pooler or DNS layer, so recovery does not require an
  application restart.
- The non-superuser application role, asserted at startup and by CI
  ([0012](../migrations/0012_application_role.sql), F-015 `Gated`).
- `/health/live` never touching the database — already designed, and worth
  re-testing after any probe configuration change.
- **Chaos test: database failover** is a named future gate
  ([15](15-CI-AND-RELEASE-GATES.md) §Future gates). Until it runs, the
  at-least-once behaviour across a real promotion is designed and unmeasured.

---

## RB-06 — Permission incident

*"Did this person really have access?"*

### Symptom

One of three, and they need different treatment:

| Trigger | Severity | What it usually is |
| --- | --- | --- |
| Permission-denied spike for one actor (10× baseline) | ticket (security) | A misconfigured integration, or a compromised account |
| Auth failure spike (10× baseline) | page (security) | Credential stuffing; see [40](40-IDENTITY-AUTH-AND-SESSION.md) |
| A customer, auditor, or legal question: "could X have read project Y between A and B?" | ticket | Neither an alert nor an outage — an evidence request |

**The two questions are not interchangeable.** `/permissions/explain` answers
*what can this actor do now*. It cannot answer *what could they do then*: the
resolver evaluates current grants, and there is no point-in-time snapshot of
effective permissions. The past is reconstructed from the audit stream, and only
within its retention window. Answering a historical question with a live
`explain` result is the most likely way to get this wrong.

### Diagnosis

**1. What is true now.** ⏳ for the endpoint (C-003, Phase 1), ✅ for the SQL.

```http
POST /api/v1/permissions/explain
{ "actor_id": "...", "permission": "task.read",
  "resource": { "type": "project", "id": "..." } }
```

Returns the decision plus the **contributing grants** or the deny reason
([04](04-RBAC-AND-AUTHORIZATION.md) §The decision function). Use it first: the
additive model means the answer is a short list of grants, never a precedence
trace, which is why it can be shown to the customer directly.

The database form of the same question — every grant that could contribute,
before constraint evaluation:

```sql
-- Reproduces principals = {actor} ∪ teams_of(actor) from docs/04 §Resolution.
SELECT ra.id, ra.principal_type, ra.principal_id, r.name AS role,
       ra.scope_type, ra.scope_id, ra.constraints,
       ra.granted_by, ra.granted_at
  FROM role_assignment ra
  JOIN role r ON r.id = ra.role_id
 WHERE ra.workspace_id = $1
   AND ( (ra.principal_type = 'USER' AND ra.principal_id = $2)
      OR (ra.principal_type = 'TEAM' AND ra.principal_id IN (
             SELECT tm.team_id
               FROM team_membership tm
               JOIN team t ON t.id = tm.team_id
              WHERE tm.user_id = $2
                AND t.workspace_id = $1
                AND t.deleted_at IS NULL)) )
 ORDER BY ra.granted_at;
```

Two things this query does **not** do, and you must do by hand:

- **Scope containment.** A `WORKSPACE`-scoped grant contains every project in the
  workspace ([04](04-RBAC-AND-AUTHORIZATION.md) §The scope containment chain).
  Rows whose `scope_id` is not the project in question may still apply.
- **Constraint evaluation.** `constraints` is a predicate over `(actor,
  resource)`. An unconstrained grant beats a constrained one; a constrained grant
  allows only when satisfied. `/permissions/explain` evaluates this; SQL does not.

And what the actor can see at all, which is a separate question from what they
can do:

```sql
SELECT p.id, p.key, p.name, p.visibility,
       (pm.user_id IS NOT NULL) AS is_member
  FROM project p
  LEFT JOIN project_membership pm ON pm.project_id = p.id AND pm.user_id = $2
 WHERE p.workspace_id = $1
   AND p.deleted_at IS NULL;
```

Visibility is evaluated before permission and produces an implicit read grant;
an invisible project returns 404, not 403
([04](04-RBAC-AND-AUTHORIZATION.md) §Visibility vs permission).

**2. What was true then.** ✅ executable (returns rows once Phase 1 writes them)

Every grant, revoke, role edit, and consent writes an `audit_event` with
before/after ([04](04-RBAC-AND-AUTHORIZATION.md) control 7):

```sql
SELECT occurred_at, event_type, actor_id, actor_type,
       target_type, target_id, changes, ip_address
  FROM audit_event
 WHERE workspace_id = $1
   AND event_type IN ('role.assigned','role.revoked','role.created','role.updated')
   AND (target_id = $2 OR changes->>'principal_id' = $2::text)
   AND occurred_at BETWEEN $3 AND $4
 ORDER BY occurred_at;
```

Reconstruct by replaying these forward from the earliest known state.

**Cost, stated honestly:** there is no stored history of *effective* permissions,
only of the grants that produce them. Reconstruction is a manual replay, it is as
complete as the audit retention window (400 days default, 90-day floor, ADR-025),
and beyond that window the question is **unanswerable**. Saying "the record does
not go back that far" is the correct answer, and it is a better answer than a
confident guess.

**3. Did they act on it?** ✅ executable

```sql
-- Uses activity_actor_ix (workspace_id, actor_id, occurred_at DESC).
SELECT occurred_at, event_type, aggregate_type, aggregate_id, project_id
  FROM activity_event
 WHERE workspace_id = $1
   AND actor_id = $2
   AND occurred_at BETWEEN $3 AND $4
 ORDER BY occurred_at;
```

Reads are not in `activity_event` — it records changes. "Did they read it" is
answerable only from request logs and traces, which carry `actor_id` and
`workspace_id` but not content ([46](46-OBSERVABILITY-AND-OPERATIONS.md) §What is
not logged). Say which of the two you are answering.

**4. Shape of the denials.** ✅ executable

```sql
SELECT date_trunc('minute', occurred_at)              AS minute,
       count(*)                                        AS denials,
       count(DISTINCT ip_address)                      AS distinct_ips,
       array_agg(DISTINCT changes->>'permission')      AS permissions
  FROM audit_event
 WHERE workspace_id = $1
   AND event_type = 'permission.denied'
   AND actor_id = $2
   AND occurred_at > now() - interval '24 hours'
 GROUP BY 1
 ORDER BY 1;
```

`permission.denied` is audited precisely because a burst of denials is the
clearest available signal of a compromised account or a misconfigured
integration, and it is invisible if only successes are recorded
([25](25-EVENTS-OUTBOX-AND-AUDIT.md) §Audit specifics).

**5. A person, a token, or an automation?** ✅ executable

`actor_type` distinguishes `USER` / `SERVICE_ACCOUNT` / `PLUGIN` / `SYSTEM`. One
permission repeatedly denied to a `SERVICE_ACCOUNT` is a missing scope; many
permissions denied to a `USER` from many IPs is a security signal.

```sql
SELECT id, name, principal_type, principal_id,
       last_used_at, expires_at, revoked_at
  FROM api_token
 WHERE workspace_id = $1
   AND principal_id = $2;
```

For an automation, `automation_rule.run_as` is the permission ceiling
([36](36-AUTOMATION-RULES-DESIGN.md)) — a rule runs as a named principal, never
as the triggering user and never as an implicit superuser. And the causal chain:

```sql
-- One query on the correlation id reconstructs the whole chain (docs/46).
SELECT occurred_at, event_type, actor_id, actor_type, target_type, target_id
  FROM audit_event
 WHERE correlation_id = $1
 ORDER BY occurred_at;
```

### Action

| Finding | Action |
| --- | --- |
| Denials are a misconfigured integration | Tell the workspace admin which scope or grant is missing. **Do not create the grant yourself.** Operators hold no workspace grants and creating one bypasses the consent and ceiling model. |
| Denials look like a compromised session | Revoke the sessions and tokens (`api_token.revoked_at`), require re-authentication, notify the workspace owner. Revocation is immediate by design, not deferred to a job ([34](34-PLUGIN-AND-EXTENSION-ARCHITECTURE.md), [40](40-IDENTITY-AUTH-AND-SESSION.md)). |
| Access was genuinely held | Say so, with the grant rows and their `granted_by` / `granted_at`. "They had it; here is who granted it and when" is the answer, comfortable or not. |
| Access was **not** held and the actor saw data anyway | Stop. Preserve evidence. Escalate as a tenancy/authorization incident ([32](32-TENANCY-AND-ISOLATION.md)). Do not remediate by deleting rows — the rows are the evidence. |
| The question is outside the audit retention window | Answer "the record does not cover that period," and record the gap. |
| It was an automation | Point at `run_as` and the correlation chain. A rule whose `run_as` principal lost access fails visibly rather than escalating silently ([36](36-AUTOMATION-RULES-DESIGN.md)). |

**Do not:**

- **Do not "fix" access by adding a deny.** There are no deny rules (ADR-004).
  Access is reduced by **removing a grant**. Improvising a deny in an ad-hoc
  query misrepresents the model to whoever reads the incident notes next.
- **Do not `UPDATE` or `DELETE` `audit_event` to redact anything.** The privilege
  does not exist for `taskforge_app` ([0012](../migrations/0012_application_role.sql)),
  and doing it as the owner destroys the only record of the incident.
- **Do not answer from the permission cache.** The `authz_epoch` cache is an
  optimization and is never the authority
  ([04](04-RBAC-AND-AUTHORIZATION.md) §Caching).
- **Do not paste `changes` payloads, IP addresses, or user agents into a shared
  ticket.** Audit content is access-controlled behind `audit.read` for a reason;
  a support ticket is not that boundary.
- **Do not conclude "no access" from an empty query result** without checking
  `current_user` and the tenant scope first (§Working on this database).

### Verification

- `/permissions/explain` for the actor, permission, and resource returns the
  expected decision and the expected grant list ⏳.
- The denial rate returns to baseline.
- If access was revoked: the actor's SSE stream closes with `403` within one
  `authz_epoch` bump, rather than surviving on a long-lived connection
  ([05](05-API-SPEC.md) §Live updates).
- If a token was revoked: `api_token.revoked_at` is set and the next call with it
  fails `TF-AUT-0010` ([20](20-ERROR-CODE-REGISTRY.md)).
- The written answer names which question it answered — "can now" or "could then".

### Prevention

- `/permissions/explain` is exposed to workspace admins, not only to operators —
  "why can't I close this?" is the most common support question in every tracker
  ([04](04-RBAC-AND-AUTHORIZATION.md) §Endpoints), and most of these incidents
  should never reach an operator.
- `permission.denied` is audited, which is what makes the spike detectable.
- Additive union with no deny rules keeps the answer short enough to show a user.
- The escalation suite — one test per privilege-escalation control, each
  attempting the exploit ([04](04-RBAC-AND-AUTHORIZATION.md) §Acceptance gates,
  [15](15-CI-AND-RELEASE-GATES.md)). ⏳ Phase 1.
- Last-owner protection and the grant/scope ceilings, enforced in-transaction.

---

## RB-07 — Restore from backup

### Symptom

Either an incident — confirmed data loss from an accidental deletion, a corrupt
volume, a bad migration, or a compromise — or the **scheduled drill**, which is a
release gate ([15](15-CI-AND-RELEASE-GATES.md) §Release gates) and is run every
phase.

A backup that has never been restored is a hypothesis about a file
([48](48-DEPLOYMENT-PROFILES.md)). The drill is what turns it into a backup.

### Diagnosis and preparation

**1. Establish the profile, the RPO, and the RTO.** ✅ executable (documentation)

| Profile | Backup | RPO | RTO |
| --- | --- | --- | --- |
| Single node | nightly `pg_dump` + attachment directory | 24 h | hours |
| Small | WAL archiving + PITR | < 5 min | < 1 h |
| Scaled | continuous + cross-region | < 1 min | < 30 min |

Owned by [48](48-DEPLOYMENT-PROFILES.md); repeated here because the first
decision in a restore is which of these you are in, and it changes the procedure.

**2. Verify the backup exists and is readable — before touching anything.**

```bash
pg_restore --list backup.dump | head -40   # fails loudly on a truncated file
ls -l /wal-archive | tail -5               # PITR: is the archive still advancing?
```

A restore that discovers the backup is unreadable *after* the primary has been
taken down is the worst outcome available in this document.

**3. Choose the restore point precisely.** ✅ executable, if the damaged database
is still readable:

```sql
SELECT occurred_at, event_type, actor_id, actor_type, target_type, target_id
  FROM audit_event
 WHERE workspace_id = $1
   AND occurred_at > now() - interval '6 hours'
 ORDER BY occurred_at DESC
 LIMIT 100;
```

Pick a timestamp immediately **before** the damaging event. Write it down. Every
later step is measured against it.

**4. Decide about attachments now, not later.** A database restore without the
matching object-store state produces rows pointing at nothing
([48](48-DEPLOYMENT-PROFILES.md)). Object storage is versioned and backed up
independently; the two restore points must be chosen together.

### Action

**Restore into a scratch instance. Never in place.** Restoring over the damaged
primary destroys the evidence and removes the ability to abort. The cost is
double storage for the duration of the restore — pay it.

**Profile 1 — `pg_dump` / `pg_restore`:** ✅ executable

```bash
# 1. Scratch instance, isolated from production networking.
docker run -d --name tf-restore \
  -e POSTGRES_USER=tf -e POSTGRES_PASSWORD=tf -e POSTGRES_DB=tf \
  -p 55433:5432 postgres:16-alpine

# 2. Restore the data. --no-owner because the dump's owner may not exist here.
pg_restore --dbname "postgres://tf:tf@127.0.0.1:55433/tf" \
  --no-owner --no-privileges --exit-on-error --jobs 4 backup.dump

# 3. THE STEP RESTORES SKIP. --no-privileges discarded every GRANT and REVOKE,
#    including the REVOKE UPDATE, DELETE that makes history append-only, and the
#    taskforge_app role the RLS policies are pointless without.
psql "postgres://tf:tf@127.0.0.1:55433/tf" -v ON_ERROR_STOP=1 \
     -f migrations/0012_application_role.sql
psql "postgres://tf:tf@127.0.0.1:55433/tf" \
     -c "ALTER ROLE taskforge_app WITH LOGIN PASSWORD '<new password>';"

# 4. Re-assert the schema invariants against the restored copy.
psql "postgres://tf:tf@127.0.0.1:55433/tf" -v ON_ERROR_STOP=1 \
     -f tests/schema/assertions.sql
```

Step 3 is the one that matters most and the one most likely to be forgotten. A
restored database whose schema is owned by a superuser with no `taskforge_app`
has **RLS and append-only history both disabled**, and nothing about it looks
wrong until a cross-tenant leak.

**Profile 2 / 3 — PITR:** ✅ executable (the PostgreSQL mechanics; the archive
itself is deployment-specific)

```
# In the scratch cluster's configuration, before first start:
restore_command       = 'cp /wal-archive/%f %p'
recovery_target_time  = '2026-08-08 09:12:00+00'
recovery_target_action = 'promote'
```

```sql
-- After it comes up: where did recovery actually stop?
SELECT pg_is_in_recovery(), pg_last_wal_replay_lsn();
SELECT max(occurred_at) AS effective_restore_point FROM audit_event;
```

`recovery_target_time` is a request; the true stopping point is the last
transaction before it. The audit timestamp above is the honest answer to "what
did we get back," and it is what goes in the incident record — not the value that
was configured.

**Attachments:** identify rows whose objects may not exist in the restored object
state. ✅ executable

```sql
SELECT count(*) AS at_risk
  FROM attachment
 WHERE committed_at IS NOT NULL
   AND deleted_at IS NULL
   AND created_at > $restore_point;
```

**Bringing the restored copy up.** Before pointing any application at it:

- `TF_WORKER_EMBEDDED=false`, or the worker disabled. A restored database
  contains **undispatched outbox rows**. Starting a worker against it re-delivers
  every one of them: webhooks fire again, notification emails send again. That is
  correct at-least-once behaviour aimed at the wrong moment.
- `TF_SMTP_*` unset or pointed at a sink until verification passes.
- A distinct `TF_PUBLIC_URL`. The production URL in a restored instance sends real
  users, and real OIDC redirects, at a scratch environment.

### Verification

A restore is **complete** when it finishes and **verified** when this table
passes. Both go in the drill record.

| Check | How | ✅/⏳ |
| --- | --- | --- |
| Schema matches the migration contract | `psql -f tests/schema/assertions.sql` (exit code 0) | ✅ |
| Application role exists and is constrained | `SELECT rolsuper, rolbypassrls FROM pg_roles WHERE rolname='taskforge_app';` → `f`, `f` | ✅ |
| Tenant isolation holds | the scoped/unscoped assertions in [`scripts/verify-schema.sh`](../scripts/verify-schema.sh) | ✅ |
| History is append-only | `UPDATE audit_event SET event_type='x';` as `taskforge_app` **must fail** | ✅ |
| Restore point is the intended one | `SELECT max(occurred_at) FROM audit_event;` | ✅ |
| Row counts are plausible | `SELECT count(*) FROM task WHERE deleted_at IS NULL;` and per-workspace counts, against the pre-incident monitoring sample | ✅ |
| Attachments resolve | sample 100 committed rows; `HEAD` each `object_key` in the restored object store | ⏳ (needs the storage client) |
| Application starts and serves | boot against the scratch DSN with workers and SMTP disabled; `/health/ready` green | ⏳ (no application binary in Phase 0) |
| Timed against the RTO | wall-clock from decision to verified, recorded | ✅ |

**A restore that meets the RPO but takes four times the RTO has failed the
drill**, even though the data is intact. Record the number and fix the procedure;
an unrecorded drill time is a drill that proves nothing.

**Do not:**

- **Do not restore over the damaged primary.** You lose the evidence and the
  option to abort.
- **Do not skip step 3** (roles and privileges). `--no-privileges` silently
  produces a database where history can be rewritten and tenants are not
  isolated.
- **Do not restore the database without deciding about attachments.** Rows
  pointing at absent objects are a second incident, discovered by users.
- **Do not start workers against a restored copy** until it is verified, or the
  outbox will re-deliver into the real world.
- **Do not delete the "duplicate" outbox rows** that appear after a cutover.
  Events created after the restore point are gone along with the transactions
  that created them — that is consistent, not loss. Events before it may be
  delivered twice, which is the contract (standing rule 2).
- **Do not skip the drill because the backup job reports success.** A green
  backup job proves a file was written. It proves nothing about whether that file
  restores.

### Prevention

- Restore drilled every phase, timed, as a release gate
  ([15](15-CI-AND-RELEASE-GATES.md), [48](48-DEPLOYMENT-PROFILES.md)).
- Migration rehearsal against a production-shaped snapshot, timed, so a bad
  migration is caught before it is a reason to restore.
- Forward-only migrations with expand → migrate → contract, contracting only in a
  **later** release — which is what makes a code rollback survivable without a
  database restore ([22](22-DATABASE-SCHEMA.md), [48](48-DEPLOYMENT-PROFILES.md)).
- Object storage backed up and versioned independently of the database.
- Partition retention that exports before dropping ([25](25-EVENTS-OUTBOX-AND-AUDIT.md)),
  so a dropped partition is not a restore event.

---

## Executability matrix

What can be followed today (Phase 0), and what is waiting on which item in
[14](14-EXECUTION-TRACKER.md).

| Runbook | Diagnosis | Action | Verification | Blocked on |
| --- | --- | --- | --- | --- |
| RB-01 Outbox lag | ✅ SQL runs; ⏳ metrics and per-consumer breakdown | ⏳ nothing to restart or scale | ⏳ metric-based | F-009, C-011, `casual-task-worker` |
| RB-02 DLQ | ✅ all SQL runs | ⏳ console replay; ✅ bounded SQL replay | ✅ SQL; ⏳ consumer metrics | F-009, C-011, admin console (Phase 2) |
| RB-03 Plugin storm | ✅ SQL runs, returns nothing until Phase 3 | ⏳ entirely | ⏳ entirely | C-011 (audit), Phase 3 plugins |
| RB-04 Search stale | ✅ staleness SQL runs | ⏳ rebuild tool does not exist | ✅ SQL; ⏳ metric | C-013, `casual-task-search` |
| RB-05 DB failover | ✅ entirely — this is the most executable runbook in the set | ✅ mostly (infrastructure, not application) | ✅ via `scripts/verify-schema.sh` | ⏳ startup superuser check (C-001 era) |
| RB-06 Permission incident | ✅ SQL runs, returns nothing until Phase 1 | ⏳ `/permissions/explain`, revocation | ⏳ endpoint-based | C-003, C-011 |
| RB-07 Restore | ✅ entirely for the database; ⏳ for the application | ✅ database; ⏳ attachment and app checks | ✅ mostly | attachment client, API binary |

Two honest summary statements:

1. **The database-level halves of all seven runbooks are executable now**, because
   the schema, the roles, and the verification script are `Gated` (F-015). That is
   most of RB-05 and RB-07 and the diagnosis half of the rest.
2. **Every action that requires observing or controlling a running system is
   not.** No metric named in [46](46-OBSERVABILITY-AND-OPERATIONS.md) is emitted
   yet, and the worker binary prints one line and exits. A runbook step that says
   "check `outbox_lag_seconds`" is a design commitment to emit that metric, and
   F-009 is where that commitment is paid.

These runbooks are re-verified at each phase gate as their ⏳ steps become
executable, alongside the periodic maintenance in
[16](16-DOCUMENTATION-MAINTENANCE.md).
