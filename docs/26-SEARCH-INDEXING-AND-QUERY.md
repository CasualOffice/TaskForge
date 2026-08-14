# 26 — Search, Indexing & Query

Everything a user can find, filter, or sort by — and the exact index that serves
it. This doc is a **contract**: if a field is not listed here, it is not
filterable, not sortable, and not searchable. Adding one means adding its index
in the same change.

The governing rule (NFR-5):

> **No user-reachable query performs a sequential scan on a tenant-scale table.**

## Why an index contract, not just "add indexes later"

Trackers die of query debt. Someone adds a filter, it works on 500 tasks, and
two years later it is a 40-second board load nobody can attribute. The failure is
not the missing index — it is that **the filterable surface was never bounded**,
so no one could enumerate what needed indexing.

TaskForge bounds it: the filterable field set is **closed and enumerated**
([27](27-FILTER-AND-SAVED-VIEW-DSL.md) is the grammar over exactly these fields).
A filter on an unlisted field is a `400`, not a slow query.

## Two search paths, one entry point

| Path | Serves | Mechanism |
| --- | --- | --- |
| **Structured filter** | boards, lists, My Work, saved views | indexed predicates over `task` |
| **Full-text** | the search box, command palette | `tsvector` + trigram over a projection |

Both are the same endpoint (`GET /api/v1/tasks`), both are permission-filtered
identically, both use the same cursor. Full-text is just another predicate:
`?q=payment+retry&status=in-progress` is one query, not a join of two systems.

## Postgres-native, and the exact tripwire for changing that

Search starts in PostgreSQL — `tsvector` + GIN for text, `pg_trgm` for fuzzy,
B-tree/GIN for structure. No Elasticsearch, no Meilisearch, in v1.

This is not "good enough for now." At the reference capacity
([30](30-PERFORMANCE-AND-CAPACITY-TARGETS.md)) — 2M tasks per workspace — a GIN
index over a materialized `tsvector` returns top-50 ranked results in single-digit
milliseconds. Postgres is the right answer, not the cheap one. It also means
search results are **transactionally consistent with permissions**, which an
external index can never guarantee without a stale-ACL window.

**The tripwire.** Introduce an external engine only when one of these is measured,
not anticipated (ADR-014):

- p95 full-text latency > 300 ms at the reference corpus, after tuning;
- a product requirement Postgres genuinely cannot serve — cross-workspace search,
  semantic/vector ranking, or per-term relevance tuning by admins;
- index maintenance cost exceeding 20% of write throughput.

If that fires, the seam already exists: the projection table below becomes the
document source, and `casual-task-search` swaps its backend. Nothing else changes.

## The search projection

Full-text does **not** run against `task` directly. A projection table carries the
search document:

```sql
CREATE TABLE task_search (
    task_id       uuid PRIMARY KEY REFERENCES task(id) ON DELETE CASCADE,
    workspace_id  uuid NOT NULL,
    project_id    uuid NOT NULL,
    document      tsvector NOT NULL,
    title_trgm    text NOT NULL,      -- title, for trigram/prefix matching
    updated_at    timestamptz NOT NULL
);
```

**Why a separate table rather than a generated column on `task`:**

1. `task` is the hot write path. A GIN index on it makes every task update pay
   GIN maintenance, and GIN's pending-list flushes are bursty — the exact latency
   spike you do not want on a drag-and-drop board.
2. The document spans **more than the task row** — title, key, description,
   comment bodies, tag names, assignee names. A generated column cannot see other
   tables.
3. It gives the external-engine seam for free.

**Maintenance:** the outbox worker refreshes the row after commit
([25](25-EVENTS-OUTBOX-AND-AUDIT.md)). Search is therefore **eventually
consistent, typically < 1 s**, while structured filters are strictly consistent
because they read `task` directly. This is the right split: a user who just typed
a title expects to *filter* to it immediately, and tolerates a beat before it is
*searchable*.

**Weighting** — `setweight` gives ranked results people expect:

| Weight | Content |
| --- | --- |
| `A` | task key (`WR-125`), title |
| `B` | tag names, assignee/reporter display names, milestone |
| `C` | description |
| `D` | comment bodies |

Query ranking uses `ts_rank_cd`, with a recency decay so a stale exact match does
not outrank a live near-match.

**The decay is not built yet (D-070).** `RANK` is bare `ts_rank_cd(s.document,
q)` today, and the sentence above has described a decay that does not exist for
long enough that a bug was raised against the symptom. It is accepted and
scheduled: the decay is computed against a reference instant captured on the
first request and **carried in the cursor**, never against `now()`, because
`RANK` is simultaneously the `ORDER BY` expression and the keyset cursor's sort
key — a rank that moves with wall-clock time makes page two disagree with page
one, which is a bug that shows up only after the first page.

## The complete index inventory

Every index, why it exists, and the query it serves. This table is the deliverable
— it is what "proper indexing of all tasks and others" means concretely.

### `task` — the hot table

| Index | Definition | Serves |
| --- | --- | --- |
| `task_pkey` | `(id)` | direct fetch |
| `task_key_uq` | `UNIQUE (project_id, number)` | human key `WR-125`; also the allocation guard |
| `task_board_ix` | `(project_id, status_id, position)` `WHERE deleted_at IS NULL` | **the board** — columns, ordering, drag targets |
| `task_list_ix` | `(project_id, updated_at DESC, id DESC)` `WHERE deleted_at IS NULL` | project list view + its cursor |
| `task_mywork_ix` | `(workspace_id, state, due_at)` `WHERE deleted_at IS NULL` | My Work buckets across projects |
| `task_assignee_ix` | on `task_assignee (user_id, workspace_id)` | "assigned to me" |
| `task_reporter_ix` | `(workspace_id, reporter_id)` `WHERE deleted_at IS NULL` | "reported by me" |
| `task_parent_ix` | `(parent_id)` `WHERE parent_id IS NOT NULL` | subtree expansion |
| `task_milestone_ix` | `(milestone_id)` `WHERE milestone_id IS NOT NULL` | milestone rollup |
| `task_env_ix` | `(project_id, environment_id)` `WHERE environment_id IS NOT NULL` | environment filter |
| `task_due_ix` | `(workspace_id, due_at)` `WHERE due_at IS NOT NULL AND deleted_at IS NULL` | overdue/upcoming sweeps + the reminder worker |
| `task_type_prio_ix` | `(project_id, type, priority)` `WHERE deleted_at IS NULL` | type/priority facets |
| `task_updated_brin` | `BRIN (updated_at)` | analytics and archival sweeps — cheap over a large append-mostly table |

Partial indexes (`WHERE deleted_at IS NULL`) matter here: soft-deleted rows are a
minority forever, and excluding them keeps every hot index smaller.

### `task_search`

| Index | Definition | Serves |
| --- | --- | --- |
| `task_search_gin` | `GIN (document)` | full-text, **including prefix** |
| `task_search_trgm` | `GIN (title_trgm gin_trgm_ops)` | typo tolerance, substring — **not yet read by anything** |
| `task_search_scope_ix` | `(workspace_id, project_id)` | permission pre-filter |

**`WR-12*` prefix moved off the trigram index (D-069).** It is served by a `:*`
on the final token through `to_tsquery`, which `task_search_gin` already
answers, so prefix costs no second index and no change to the query's plan
shape. That mattered: `@@` is a non-`LEAKPROOF` `ts_match_vq` under row-level
security (**D-043**), so an `OR` across two indexes is a plan change to be
measured rather than assumed. Measured both ways, the `explain-no-seq-scan`
gate reports 29 index-served and 0 sequential scans with an identical advisory
list.

Only the **last** token is a prefix — it is the one being typed;
`restore backu` compiles to `restore & backu:*`. The term is reduced to
alphanumerics and `-` before it reaches `to_tsquery`, because that function
parses its argument as tsquery *syntax*: `&`, `|`, `!`, `(`, `)` and `:` in
somebody's typing are operators unless they never arrive.

`task_search_trgm` is therefore **written on every task and read by nothing**
until typo tolerance lands. That is stated rather than left for a reader to
discover from the absence of a query: it is a real write cost for a capability
that does not exist yet, and D-069 part two decides whether it earns its keep or
is dropped.

### `task_tag` (many-to-many)

| Index | Definition | Serves |
| --- | --- | --- |
| `task_tag_pkey` | `(task_id, tag_id)` | tags of a task |
| `task_tag_rev_ix` | `(tag_id, task_id)` | **tasks of a tag** — the reverse direction a composite PK alone does not serve |

The reverse index is the classic omission. Without it, "show everything tagged
`security`" scans.

### `activity_event` / `audit_event`

| Index | Definition | Serves |
| --- | --- | --- |
| `activity_stream_ix` | `(workspace_id, aggregate_id, occurred_at DESC)` | a task's history tab |
| `activity_project_ix` | `(project_id, occurred_at DESC)` | project activity feed |
| `activity_actor_ix` | `(workspace_id, actor_id, occurred_at DESC)` | "what did this person do" |
| `activity_brin` | `BRIN (occurred_at)` | retention/partition pruning |
| `audit_ix` | `(workspace_id, occurred_at DESC)` + `(workspace_id, event_type, occurred_at DESC)` | compliance export, security review |

Both are append-only and range-partitioned monthly, so retention is a partition
drop rather than a `DELETE` of millions of rows.

### Authorization tables — read on every request

| Index | Definition | Serves |
| --- | --- | --- |
| `role_assignment_lookup_ix` | `(workspace_id, principal_type, principal_id, scope_type, scope_id)` | **the resolver's hot path** ([04](04-RBAC-AND-AUTHORIZATION.md)) |
| `role_assignment_scope_ix` | `(workspace_id, scope_type, scope_id)` | "who has access to this project" |
| `role_permission_ix` | `(role_id)` | expanding a role |
| `project_membership_ix` | `(user_id, workspace_id)` and `(project_id, user_id)` | both directions of membership |
| `team_membership_ix` | `(user_id)` and `(team_id, user_id)` | principal expansion |

### Outbox & workers

| Index | Definition | Serves |
| --- | --- | --- |
| `outbox_delivery_pending_ix` | `(consumer, next_attempt_at, created_at)` `WHERE dispatched_at IS NULL AND dead_lettered_at IS NULL` | the dispatch poll. Led by `consumer` because a worker polls for exactly one — a time-leading index makes it walk five other consumers' due rows to reach its own. Both partial predicates matter: dispatched rows leave it so it stays tiny, and dead-lettered rows leave it so a growing DLQ — whose rows are by definition the *oldest* pending ones — cannot sit at its head and be re-read on every poll |
| `outbox_event_aggregate_ix` | `(aggregate_id, created_at, id)` | the claim query's per-aggregate ordering anti-join ([25](25-EVENTS-OUTBOX-AND-AUDIT.md) §Delivery semantics): "is there an earlier undelivered event for this aggregate?". Without it the planner answers that by hashing every pending delivery for the consumer on every poll — no sequential scan, so the plan gate passed, and the cost was O(pending) to claim a batch of 64. The trailing `id` is what lets the row-wise `(created_at, id) <` comparison be an index bound rather than a filter |
| `outbox_delivery_dlq_ix` | `(consumer, workspace_id, dead_lettered_at)` `WHERE dead_lettered_at IS NOT NULL` | `outbox_dlq_depth` and dead-letter review (RB-02). Led by `consumer` so the gauge's `GROUP BY consumer` is an index-only scan; the original definition carried no `consumer` column, so the count paid one random heap read per dead row — over a set that is deliberately never swept and therefore only grows |
| `outbox_delivery_retention_ix` | `(dispatched_at, id)` `WHERE dispatched_at IS NOT NULL` | the seven-day delivery sweep in oldest-first batches. The partial predicate excludes pending and dead-letter rows and supplies the same predicate the delete uses, so a cleanup query cannot walk the live queue |
| `outbox_event_retention_ix` | `(created_at, id)` | the orphan-event half of the seven-day sweep. The order makes each batch deterministic and bounded; `outbox_delivery_event_id_consumer_key` serves the correlated existence check that keeps an event while any delivery remains |
| `notification_unread_ix` | `(user_id, created_at DESC)` `WHERE read_at IS NULL` | the inbox badge |
| `notification_inbox_ix` | `(user_id, (read_at IS NULL) DESC, created_at DESC, id DESC)` | the inbox page. [29](29-NOTIFICATIONS-AND-DELIVERY.md) orders it "with unread first", which makes the unread flag the **leading** sort key — something the partial index above cannot serve, because it holds only half the page. `(read_at IS NULL)` is `IMMUTABLE` and therefore indexable, so the order costs no sort (migration 0024, C-016) |
| `notification_coalesce_ix` | `(user_id, aggregate_id, created_at DESC)` `WHERE read_at IS NULL` | the 5-minute coalescing lookup ([29](29-NOTIFICATIONS-AND-DELIVERY.md) rule 2), which runs on **every** event the fan-out delivers. Without it, a scan of the recipient's whole unread set each time (migration 0024, C-016) |

### Everything else

| Index | Definition | Serves |
| --- | --- | --- |
| `project_ws_ix` | `(workspace_id, archived_at)` | project list |
| `project_list_ix` | `(workspace_id, created_at DESC, id DESC)` `WHERE deleted_at IS NULL` | the project list's **cursor** — `project_ws_ix` supplies the tenant filter but no order, so every page would sort without this. Same shape as `task_list_ix`, one level up (migration 0019, C-006) |
| `workflow_default_uq` | `UNIQUE (workspace_id)` `WHERE is_default` | one default workflow per workspace. The first project create in a workspace provisions it ([23](23-WORKFLOW-AND-STATE-MACHINE.md) §The default workflow), and two concurrent first creates would otherwise each insert one with no error anywhere (migration 0019, C-006) |
| `project_key_uq` | `UNIQUE (workspace_id, key)` | key allocation |
| `tag_scope_uq` | `UNIQUE (workspace_id, project_id, name)` | tag uniqueness + typeahead |
| `comment_task_ix` | `(task_id, created_at)` | comment thread |
| `attachment_task_ix` | `(task_id)` `WHERE deleted_at IS NULL` | files tab |
| `saved_view_ix` | `(workspace_id, owner_id)` | saved view list |
| `idempotency_uq` | `UNIQUE (workspace_id, actor_id, key)` | retried creates ([24](24-CONCURRENCY-AND-IDEMPOTENCY.md)) |

## Permission filtering — the hard part, solved simply

The classic failure is searching first and filtering by permission after: page
sizes collapse, cursors break, and result counts lie.

TaskForge filters **before** ranking, using the fact that permissions do not vary
within a project (ADR-005):

```sql
SELECT t.id, ts_rank_cd(s.document, q) AS rank
  FROM task_search s
  JOIN task t ON t.id = s.task_id, plainto_tsquery('english', $2) q
 WHERE s.workspace_id = $1
   AND s.project_id = ANY($3)        -- ← the actor's accessible project set
   AND s.document @@ q
   AND t.deleted_at IS NULL
 ORDER BY rank DESC, t.id DESC
 LIMIT 51;                            -- 51 to detect "has next page"
```

`$3` comes from `accessible_projects(actor)`, resolved once and cached per
`authz_epoch` ([04](04-RBAC-AND-AUTHORIZATION.md)). Typical cardinality is tens;
`= ANY(array)` on an indexed column handles that efficiently.

**The escape hatch, pre-designed:** if a workspace ever exceeds ~2,000 accessible
projects for one actor, the array stops being efficient. At that point
`accessible_projects` is materialized into a `project_access(user_id, project_id)`
table refreshed on `authz_epoch` change, and `= ANY(...)` becomes a join. Same
query shape, same semantics — a capacity fix, not a redesign. Threshold and
trigger are in [30](30-PERFORMANCE-AND-CAPACITY-TARGETS.md).

## Cursor pagination

Offset pagination is banned. It scans, and it duplicates or skips rows under
concurrent writes — both of which a live board guarantees.

Every cursor is an opaque base64 of the **sort key plus the tiebreaker id**:

```
cursor = base64({ "k": [<sort field values...>], "id": "<uuid>" })

... WHERE (sort_field, id) < ($cursor_k, $cursor_id)
    ORDER BY sort_field DESC, id DESC
    LIMIT $n + 1
```

The `id` tiebreaker is mandatory — without it, ties in `updated_at` (which happen
constantly on bulk operations) make the cursor non-deterministic.

**Sortable fields are a closed set**, each backed by an index above:

`created_at` · `updated_at` · `due_at` · `priority` · `status.position` ·
`position` (board rank) · `key` · `rank` (full-text only)

A sort on anything else is `400 TF-QRY-0002`. This is what makes NFR-5 enforceable
rather than aspirational.

## Board ordering

Manual card order uses a **lexicographic rank string** (`position text`), not a
float or an integer.

Floats run out of precision after ~50 drags between the same pair of cards, at
which point ordering silently corrupts. Integers require renumbering the column
on every insert. Lexicographic ranks (`"a0"`, `"a0V"`, `"a1"`) insert between any
two neighbours by generating a midpoint string, need no renumbering, and sort
correctly with a plain B-tree. A background compaction job shortens ranks that
grow past 32 chars. See ADR-013.

## Query limits

Enforced at the edge; full list in [21](21-API-LIMITS-AND-QUOTAS.md).

| Limit | Value | Why |
| --- | --- | --- |
| Page size | 100 (default 50) | bounds work per request |
| Filter clauses | 32 | bounds planner cost |
| Filter nesting depth | 4 | bounds recursion |
| Search term length | 256 chars | bounds `tsquery` construction |
| `IN` list length | 100 | bounds index probes |
| Statement timeout | 5 s (2 s for search) | nothing runs away |

Every rejection returns a `TF-QRY-*` code ([20](20-ERROR-CODE-REGISTRY.md)) naming
the exceeded limit — never a generic 400.

## Acceptance gates

- **`EXPLAIN` assertion suite** — every endpoint × every sortable field, asserting
  `Index Scan`/`Bitmap Heap Scan` and **no `Seq Scan`** on any tenant-scale table
  (the list is `tests/explain/tenant-scale-tables.txt`, which covers `task`,
  `task_search`, `activity_event`, and `role_assignment` among others). Runs in
  CI as the `explain-no-seq-scan` job, against a seeded corpus, planned as the
  non-superuser `taskforge_app` so the RLS predicate is in the plan.
- **Reference corpus** — 2M tasks / 200 projects / 500 users, generated
  deterministically by `tools/casual-task-seed` as PostgreSQL `COPY` files. It is
  *generated*, not committed: at reference scale it is ~10.5 GiB of text. The
  `EXPLAIN` gate therefore runs against a reduced corpus (`tests/explain/seed.sql`,
  ~109k tasks) which proves plan **shape**; the reference corpus is for the
  latency gates ([30](30-PERFORMANCE-AND-CAPACITY-TARGETS.md) §Measurement).
- **Latency gates** — p95 board load < 150 ms, full-text < 200 ms, My Work
  < 200 ms at reference scale ([15](15-CI-AND-RELEASE-GATES.md)).
- **Cursor property test** — for random insert/update interleavings, paging the
  full set yields every row exactly once.
- **Permission-filter test** — search never returns a task from an inaccessible
  project, including for tasks whose text matches strongly.
- **Rank-stability test** — 10,000 random board reorders keep ranks unique,
  correctly ordered, and under the length cap.

## ADRs triggered

- **ADR-011** — Closed filterable/sortable field set with a named index each.
- **ADR-013** — Lexicographic rank strings for manual ordering.
- **ADR-014** — PostgreSQL-native search, with the measured tripwire for revisiting.
