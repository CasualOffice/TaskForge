# 38 — Reporting, Export & Dashboards

The analytics surface. The old drafts listed "reporting projections" as a Phase 4
line item with no design, and [18](18-SUPPORT-MATRIX.md) marked it *not designed*
— which meant it could not be promised to anyone. This closes that.

Three capabilities, one substrate:

| | What | Phase |
| --- | --- | --- |
| **Export** | Get my filtered data out, in a format I can use elsewhere | 2 |
| **Reports** | Answer a fixed operational question — cycle time, throughput, workload | 4 |
| **Dashboards** | Compose saved reports into a page I check every morning | 4 |

## The governing constraint

Analytics is where trackers quietly become slow. A report is a query nobody
bounded, over a table nobody indexed for it, run by everyone at 9am.

> **A report is a saved filter plus an aggregation, over the same closed field
> set as everything else** ([26](26-SEARCH-INDEXING-AND-QUERY.md),
> [27](27-FILTER-AND-SAVED-VIEW-DSL.md)).

No report defines its own query. No report reaches a field the filter grammar
cannot express. This is what keeps the index contract (ADR-011) true when
reporting arrives, rather than making reporting the exception that breaks it.

The simplicity contract applies too ([01](01-ORD.md)): reporting adds **no new
user-facing noun**. A report is a saved view with an aggregation; a dashboard is
a page of them. "Saved view" is already in the glossary.

## Export

The capability people actually ask for first, and the cheapest to get right.

### What can be exported

| Export | Contents | Permission |
| --- | --- | --- |
| **Task list** | The current filter's results, chosen columns | `task.read` on the projects in scope |
| **Task detail** | Tasks + comments + activity + attachment metadata | `task.history.read` |
| **Audit** | The audit stream for a period | `audit.read` |
| **Workspace** | Everything, for migration or backup | `workspace.manage` |

### Formats

| Format | For | Notes |
| --- | --- | --- |
| **CSV** | spreadsheets, the 90% case | RFC 4180, UTF-8 with BOM so Excel opens it correctly |
| **JSON Lines** | pipelines and scripts | one object per line, streamable |
| **XLSX** | stakeholders who will not accept CSV | generated through **OpenCalc**, the suite's own engine — no new dependency |

Using OpenCalc for XLSX is the one place the suite's shared architecture pays a
direct dividend. It is a Rust crate we already maintain, so `.xlsx` export costs
a dependency edge rather than a new vendor.

**PDF is deliberately not an export format for data.** A PDF of 5,000 rows helps
nobody. PDF belongs to *report rendering* (below), where layout is the point.

### Export is a job, not a request

Anything above 1,000 rows is asynchronous:

```
POST /api/v1/exports        { filter, format, columns }  → 202 { export_id }
GET  /api/v1/exports/{id}                                → status + progress
GET  /api/v1/exports/{id}/download                       → 302 to a signed URL
```

- Runs on the worker, streaming rows in cursor-paginated batches straight to
  object storage. **The API process never holds the result set in memory** —
  the same discipline as attachments ([28](28-ATTACHMENT-PIPELINE.md)).
- **Permissions are evaluated per batch, not once at the start.** A long export
  must not keep emitting rows from a project the actor lost access to halfway
  through. The compiled filter carries the permission predicate
  ([27](27-FILTER-AND-SAVED-VIEW-DSL.md)), and `authz_epoch` changes force
  re-resolution.
- Signed download URL, 1 hour, single-use where the backend supports it.
- Artifacts are deleted after 7 days.
- **Every export writes an `audit_event`** with the filter, row count, and
  format. Bulk data leaving the system is exactly what an audit trail is for.

### The CSV injection problem

A cell beginning `=`, `+`, `-`, or `@` is executed as a formula when the file is
opened in Excel or Sheets. A task titled `=cmd|'/c calc'!A1` becomes remote code
execution on a colleague's laptop, and the attacker only needed permission to
create a task.

Every exported cell whose first character is one of those is prefixed with a
single quote. This is non-negotiable and has its own test — it is the single most
commonly shipped export vulnerability.

## Reports

### The report model

```jsonc
{
  "name": "Cycle time by assignee, last 30 days",
  "source": "task",
  "filter":  { "op": "and", "clauses": [ ... ] },   // the SAME grammar, doc 27
  "measure": { "fn": "p50", "of": "cycle_time" },
  "group_by": ["assignee"],
  "bucket":   { "field": "completed_at", "interval": "week" },
  "limit":    20
}
```

Four parts: **filter** (which tasks), **measure** (what number), **group_by**
(which slices), **bucket** (which time grain). Nothing else. A report that needs
a fifth part is a signal the model is wrong, not that the report is special.

### Measures — a closed set

| Measure | Definition |
| --- | --- |
| `count` | number of tasks |
| `sum` / `avg` / `p50` / `p90` of a numeric field | over the matched set |
| `cycle_time` | first entry to an `ACTIVE` state → first entry to `COMPLETED` |
| `lead_time` | `created_at` → first entry to `COMPLETED` |
| `age` | `created_at` → now, for open tasks |
| `time_in_state` | duration in a given state |
| `throughput` | count entering `COMPLETED` per bucket |
| `created_vs_completed` | two series, per bucket |

**`CANCELED` never counts as completed.** Cycle time and throughput exclude it
entirely. Collapsing the two is the most common metric bug in trackers, and it is
precisely why `CANCELED` is a separate state ([23](23-WORKFLOW-AND-STATE-MACHINE.md)).

### Where the numbers come from

Cycle time is not a column. It is derived from state-transition history, and
computing it by scanning `activity_event` at query time would be exactly the
unbounded query this document exists to prevent.

A **projection** maintained by the outbox worker
([25](25-EVENTS-OUTBOX-AND-AUDIT.md)):

```sql
CREATE TABLE task_state_interval (
    task_id       uuid NOT NULL REFERENCES task(id) ON DELETE CASCADE,
    workspace_id  uuid NOT NULL,
    project_id    uuid NOT NULL,
    state         task_state NOT NULL,
    status_id     uuid NOT NULL,
    entered_at    timestamptz NOT NULL,
    exited_at     timestamptz,                 -- NULL = current
    duration      interval GENERATED ALWAYS AS (exited_at - entered_at) STORED,
    PRIMARY KEY (task_id, entered_at)
);
CREATE INDEX tsi_cycle_ix ON task_state_interval (workspace_id, project_id, state, entered_at);
CREATE INDEX tsi_open_ix  ON task_state_interval (task_id) WHERE exited_at IS NULL;
```

One row per state occupancy. Cycle time becomes a bounded aggregate over an
indexed table, and "how long was this stuck in Code Review?" becomes answerable —
a question the raw activity stream can answer only by replay.

It is **rebuildable from `activity_event`**, which is append-only and complete.
A projection that cannot be rebuilt is a second source of truth; this one is a
cache.

### What is built today (C-026)

`POST /api/v1/reports/run` takes the URL-form filter, one dimension from the
closed set, and returns a grouped `count` with the project scope it was computed
over. The filter is parsed, resolved, validated and compiled by the **list
query's own pipeline** — `compile_group_count` sits beside `compile` in the same
module, so the tenant predicate and the authorized project set are injected in
exactly one place.

`cycle_time`, `lead_time` and `throughput` joined it in C-030 and `age` in
C-043, `created_vs_completed` and `time_in_state` in C-044 and C-046. Every measure this document names is now built except the `sum`/`avg`/`p50`/`p90` of an arbitrary numeric field, which no field in the schema needs yet; anything outside the set is still refused **by name** rather than approximated. Saved reports are
not built — a run is ad-hoc, and its "saved" form is the URL the toolbar already
produces. Dashboards ship as the four built-ins (C-035, §Dashboards below).

### Report execution limits

Same discipline as every other query ([21](21-API-LIMITS-AND-QUOTAS.md)):

| Limit | Value |
| --- | --- |
| Time range | 2 years |
| Result groups | 1,000 |
| Buckets | 400 |
| Statement timeout | 10 s (higher than search; still bounded) |
| Concurrent reports per workspace | 5 |
| Cached result TTL | 5 min |

Reports read from a **read replica** where one exists
([48](48-DEPLOYMENT-PROFILES.md)), and shed first under load
([30](30-PERFORMANCE-AND-CAPACITY-TARGETS.md)). A slow report must never slow a
board.

### Permissions

A report shows only what the viewer can see — the permission predicate is
injected by the compiler, identically to a list query. Two people opening the
same report see different numbers, and that is correct.

**This has a consequence worth stating plainly:** aggregate numbers are not
comparable between viewers. A manager's "47 open" and a guest's "12 open" are
both right. Reports therefore display the project scope they were computed over,
so nobody quotes a number without knowing whose view produced it.

## Dashboards

A dashboard is a **named layout of saved reports**. That is the whole model.

```jsonc
{
  "name": "Team Monday",
  "scope": { "project_ids": [...] },
  "tiles": [
    { "report_id": "...", "viz": "number",   "w": 1, "h": 1 },
    { "report_id": "...", "viz": "line",     "w": 2, "h": 2 },
    { "report_id": "...", "viz": "bar",      "w": 2, "h": 2 },
    { "report_id": "...", "viz": "table",    "w": 4, "h": 3 }
  ],
  "refresh": "5m"
}
```

- **Visualizations are a closed set**: `number`, `line`, `bar`, `donut`,
  `stacked_bar`, `table`, `heatmap`. No chart builder, no arbitrary viz config.
  Seven well-made charts beat a chart builder nobody can use.

  `donut` was added to the set after the built-ins shipped, on request, and is
  recorded here rather than left as a divergence in the client. It earns a place
  because composition is a question a stacked bar answers badly: a bar is read
  as a *length* and invites comparing segments to each other, while a ring is
  read as a *proportion* and answers "how much of the open work is this". The
  hole carries the total, which is the number people want beside the shares.
- Tiles load **independently and lazily**. One slow report degrades its own tile,
  not the page.
- The dashboard route is a **lazy chunk**; the charting library is not in the core
  bundle ([42](42-FRONTEND-ARCHITECTURE.md), ADR-024).
- Refresh is polled per tile, honouring the cache TTL — dashboards left open on a
  wall display must not become a load generator.

### Built-in dashboards

Shipped, expressed entirely in the model above — which is the proof the model is
sufficient:

| Dashboard | Tiles |
| --- | --- |
| **My Week** | open by state · overdue count · completed this week · upcoming |
| **Project Health** | throughput trend · cycle-time p50/p90 · created vs completed · blocked count · age of oldest open |
| **Team Workload** | open per assignee · overdue per assignee · unassigned |
| **Quality** | bugs opened vs closed · bug age · reopen rate |

If a built-in dashboard needed a capability the model lacks, that is the signal
the model is under-specified. That is why they are defined this way rather than
hand-built.

### What is built today (C-035)

All four dashboards ship, as **data** — `webapp/src/views/dashboard/builtin.ts`
holds nothing but `filter` + `measure` + `group_by` + `bucket` per tile, posted
to the same `POST /reports/run` a user-composed dashboard will use. There is no
private path a built-in tile can take that a saved report could not.

Writing them out did what this section predicted it would, and the model came
up short in four places. They are **absent rather than approximated** — a wrong
number on a dashboard gets quoted in a meeting, where a missing one gets asked
about:

| Tile | Blocked on |
| --- | --- |
| Created vs completed | `created_vs_completed` — two series in one answer |
| Reopen rate | a measure over `COMPLETED → ACTIVE` transitions |

Six of the seven visualizations are built: `number`, `bar`, `line`, `donut`,
`stacked_bar`, `table`. `heatmap` is not, because no built-in tile needs one and
a menu is a promise.

**Every tile that counts tasks is a link to those tasks.** The tile's own filter
is the list's address — `searchFromFilter` is the inverse of the translation
every view already uses, so the count and the rows behind it are the same
clause and cannot disagree. This is what makes the surface a workflow rather
than a wall of numbers: notice, open, act. A *duration* tile is deliberately not
a link — "cycle time by project" measures completed work, and a list behind it
would have a row count with no relationship to the number above it.

**Tiles are ranked, not uniform.** `number` tiles are *signals* and render in
their own band above the charts, larger, and coloured by an `Intent` the tile
declares: `alert` for a commitment already missed, `watch` for work stalled or
unowned, `plain` for size. A signal at zero renders calm whatever its intent —
colouring "0 overdue" red for its category trains people to stop reading the
colour. No thresholds are invented: the product says what *kind* of number it
is, never that 20 is fine and 21 is not.

The first version of this surface had none of that — nine equal cards, each
with two lines of explanatory prose above a small number, none of them
clickable. It read as a page of text rather than a dashboard, which is the
failure this section now exists to prevent.

**There is no charting library.** The closed visualization set is what makes
that possible — six shapes is a set you can draw, and the whole dashboard route
including its stylesheet is 5.1 KiB gzip against the ~95 KiB a chart library
would have cost. Every chart renders its SVG `aria-hidden` beside a
visually-hidden `<table>` of the same numbers, per [47](47-TASK-SURFACE-TEMPLATE.md):
the drawing is decoration, the table is the content.

Saved reports and user-composed dashboards are still not built. A dashboard is
selected by URL (`/dashboards/{id}`) and its tiles are fixed.

**Tile concurrency is bounded in the browser.** Nine tiles mounting together
sent nine reports at once and a run of them came back `503`. §Report execution
limits caps concurrency at 5 per workspace, and the edge's answer to breaching
it is a refusal — which renders as an error in a tile whose number was
perfectly computable. So the client queues its own tiles at 4, leaving a slot
for a second tab.

## What this deliberately is not

- **Not a BI tool.** No joins the user defines, no custom SQL, no pivot builder,
  no calculated fields. Those needs are real and are served by **exporting to a
  real BI tool**, which is why export ships two phases earlier than reports.
- **Not real-time.** Projections lag by seconds; dashboards cache for minutes.
  A tracker's analytics do not need to be live, and pretending otherwise costs
  the write path.
- **Not cross-workspace.** Ever ([32](32-TENANCY-AND-ISOLATION.md)).
- **Not a data warehouse.** For long-horizon analysis, export or stream events to
  the customer's own warehouse via webhooks
  ([34](34-PLUGIN-AND-EXTENSION-ARCHITECTURE.md)).

## Delivery

| Phase | Ships |
| --- | --- |
| **2** | Export: CSV + JSONL, async jobs, per-batch permission checks, injection escaping, audit |
| **2** | Audit export ([25](25-EVENTS-OUTBOX-AND-AUDIT.md)) |
| **4** | `task_state_interval` projection + rebuild |
| **4** | Reports: measures, grouping, bucketing, caching |
| **4** | Dashboards: tiles, six visualizations, built-ins |
| **4** | XLSX export via OpenCalc |
| later | Scheduled report delivery by email; report subscriptions |

Export first, deliberately. It is the capability with the highest ratio of demand
to complexity, and shipping it early relieves most of the pressure for a
half-built BI tool.

## Acceptance gates

- **CSV injection test** — tasks titled `=1+1`, `+x`, `-x`, `@x`, and
  `=cmd|'/c calc'!A1` all export quote-prefixed. Non-negotiable.
- **Streaming test** — exporting 500,000 rows leaves worker RSS flat.
- **Mid-export revocation test** — an actor who loses project access partway
  through a long export gets no further rows from that project.
- **Cross-tenant test** — no export, report, or dashboard tile returns a row from
  another workspace, including via a shared report.
- **Metric correctness fixtures** — hand-calculated cycle time, throughput, and
  reopen rate over a golden task history; the numbers must match exactly.
- **`CANCELED` exclusion test** — canceled tasks never appear in throughput or
  cycle time.
- **Projection rebuild test** — dropping and rebuilding `task_state_interval`
  from `activity_event` reproduces identical numbers.
- **`EXPLAIN` suite** — every report query is index-served at the reference
  corpus ([26](26-SEARCH-INDEXING-AND-QUERY.md)).
- **Bundle test** — the charting library is absent from the core shell chunk.
- **Tile isolation test** — one failing report degrades its tile only.

## ADRs triggered

- **ADR-027** — Reports are saved filters plus a closed measure set; no
  user-defined SQL, no BI query builder.
- **ADR-028** — `task_state_interval` projection as the metric substrate,
  rebuildable from `activity_event`.
- **ADR-029** — Export is asynchronous above 1,000 rows, permission-checked per
  batch, and always audited.
- **ADR-030** — XLSX export via OpenCalc rather than a third-party writer.
