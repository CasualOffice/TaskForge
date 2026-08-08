# 23 — Workflow & State Machine

Configurable statuses over five permanent states. This is the mechanism that lets
teams model their process without every downstream consumer having to understand
their vocabulary.

## The two-layer model

```
   ┌──────────────────────────────────────────────────────────┐
   │  STATUS  — configurable, renameable, per workflow         │  what users see
   │  Backlog · Todo · In Progress · Code Review · Blocked ·   │
   │  Ready for QA · Done · Canceled                           │
   └───────────────────────┬──────────────────────────────────┘
                           │  each maps to exactly one
   ┌───────────────────────┴──────────────────────────────────┐
   │  STATE  — closed enum, permanent, in the API forever      │  what code sees
   │  BACKLOG · PLANNED · ACTIVE · COMPLETED · CANCELED        │
   └──────────────────────────────────────────────────────────┘
```

**Why the split matters more than it looks.** Without it, every report, every
automation, every plugin, and every "is this done?" query has to know a
particular team's status names. Rename "Done" to "Shipped" and reporting breaks
across the workspace. With it, a plugin asks `state == COMPLETED` and is correct
in every workspace forever.

**State is a closed enum for the life of the API.** Adding a sixth state is a
breaking change requiring a major API version. This is the single most important
stability guarantee in the product, and it is why five were chosen carefully:

| State | Means | Typical statuses |
| --- | --- | --- |
| `BACKLOG` | Captured, not committed to | Backlog, Icebox, Triage |
| `PLANNED` | Committed, not started | Todo, Ready for Development, Scheduled |
| `ACTIVE` | Being worked, including stalled work | In Progress, Code Review, Blocked, Ready for QA |
| `COMPLETED` | Finished successfully | Done, Shipped, Closed |
| `CANCELED` | Terminated without completion | Canceled, Won't Do, Duplicate |

**`Blocked` is `ACTIVE`, not its own state.** Blocked work is committed work
that is stalled; making it a state would mean every consumer handles a sixth
case, and cycle-time reports would lose the fact that the clock is still running.
Blockage is visible through the status name and through `task_dependency`.

**`CANCELED` is separate from `COMPLETED`** because throughput and cycle-time
metrics must not count abandoned work as delivered. Collapsing them is the most
common metric bug in trackers.

## Workflow structure

A **workflow** is a set of statuses plus the allowed transitions between them. A
project has exactly one. Workflows are workspace-level objects and may be shared
by many projects.

```
workflow
  ├── workflow_status   (name, state, position, is_initial)
  └── workflow_transition (from_status_id | NULL, to_status_id,
                           required_permission, required_fields,
                           ignore_dependencies)
```

- Exactly **one** status per workflow has `is_initial` (enforced by a partial
  unique index, [22](22-DATABASE-SCHEMA.md)).
- `from_status_id IS NULL` means **from any status** — how "Cancel from anywhere"
  is expressed without n rows.
- A transition may require a permission, may require fields to be non-empty, and
  may opt out of dependency gating.

## The default workflow

Works with zero configuration. Most teams never change it.

```
        ┌─────────┐      ┌──────┐      ┌─────────────┐      ┌──────┐
        │ Backlog │ ───▶ │ Todo │ ───▶ │ In Progress │ ───▶ │ Done │
        │ BACKLOG │ ◀─── │PLANNED│ ◀─── │   ACTIVE    │ ◀─── │COMPLETED│
        └─────────┘      └──────┘      └──────┬──────┘      └──────┘
                                              │ ▲
                                        ┌─────▼─┴────┐
                                        │  Blocked   │
                                        │   ACTIVE   │
                                        └────────────┘

        ── from any status ──▶  ┌──────────┐
                                │ Canceled │  CANCELED  (terminal)
                                └──────────┘
```

Backward transitions are permitted by default; reopening from `Done` requires
`task.reopen`.

## The transition command

Status is **never** written through `PATCH /tasks/{id}`. Attempting it returns
`400 TF-WFL-0001`. The only door:

```http
POST /api/v1/tasks/{id}/transitions
If-Match: "7"
Content-Type: application/json

{ "to_status_id": "...", "fields": { "resolution": "fixed" }, "comment": "..." }
```

This is not ceremony. A status field write would bypass transition validity,
required fields, dependency gating, transition permissions, and the automation
hook — all of which are the reasons a workflow exists.

### Validation order (fixed, and observable)

Checks run in this order, and the **first failure is the one reported** — so the
error a user sees is the most actionable one, not whichever check happened to run
first:

1. **Task readable** → else `404`.
2. **Version matches `If-Match`** → else `409` with current representation
   ([24](24-CONCURRENCY-AND-IDEMPOTENCY.md)).
3. **Actor holds `task.transition`** on the project → else `403`.
4. **Transition edge exists** in the workflow → else `422 TF-WFL-0002`.
5. **Transition's `required_permission`** held → else `403 TF-WFL-0003`.
6. **Required fields** present and non-empty → else `422 TF-WFL-0004`, naming
   every missing field at once (not one per round-trip).
7. **Blocking dependencies** resolved, unless `ignore_dependencies` or the actor
   holds `task.dependency.override` → else `422 TF-WFL-0005`, naming the blockers
   the actor can see.
8. **Plugin `validation.transition`** hooks, 500 ms, fail-open (ADR-017) → else
   `422 TF-PLG-0001` with the plugin's message.

Steps 3–8 are cheap and run against already-loaded data — the whole check is one
round trip to the database.

### What commits

One transaction:

```sql
UPDATE task
   SET status_id = $new, state = $new_state, version = version + 1,
       updated_at = now(), updated_by = $actor
 WHERE id = $id AND version = $expected;      -- 0 rows ⇒ 409

INSERT INTO activity_event (...);             -- "moved In Progress → Done"
INSERT INTO audit_event (...);                -- + ip, ua, request id
INSERT INTO outbox_event (...);               -- task.status.changed
INSERT INTO comment (...);                    -- only if one was supplied
```

`state` is written in the same statement as `status_id`, so the derived column
can never drift. This is the invariant that lets every report read `state`
without a join.

## Closing and reopening

Not special-cased. "Close" means *transition to a status whose state is
`COMPLETED`*, which requires:

- `task.close` **and** a valid transition edge; both, not either.

"Reopen" means transitioning **out of** a terminal state (`COMPLETED` or
`CANCELED`), requiring `task.reopen` and a valid edge. Reopening writes a
distinct `task.reopened` event, because "how often does work come back?" is a
question teams need answered and a generic status-change event cannot serve.

## Editing a workflow — the gap the old drafts left

The old drafts said workflows are configurable and never said what happens to
in-flight work. Settled:

### Deleting a status

A status holding tasks **cannot be deleted**. The admin must supply a
**migration target**:

```http
DELETE /api/v1/workflows/{wid}/statuses/{sid}?migrate_to={other_sid}
```

Then, in one transaction: every task on the deleted status moves to the target,
each writes an activity event attributed to the acting admin with reason
`workflow_migration`, and the status row is removed. Bulk moves above 10,000
tasks run as a tracked background job with progress, not a request.

Silently orphaning tasks, or lazily remapping them on next read, are both
rejected — they produce tasks whose history does not explain their status.

### Changing a status's state mapping

Permitted, and **retroactive by construction**: `task.state` is recomputed for
every task on that status in the same transaction. This visibly changes historical
reports, so the operation requires `project.workflow.manage`, writes a
prominent audit event, and warns in the UI with the affected task count.

### Removing a transition

Allowed freely — it constrains future moves only. Tasks are never in a
transition, only in a status.

### Changing a project's workflow

The heaviest operation. Requires an explicit status-by-status mapping from the
old workflow to the new one; the API rejects a partial mapping. Executed as a
background job, idempotent, resumable, with a per-task activity record.

## Concurrency

Two users transitioning the same task race on `version`. The loser gets `409`
with the current representation, and the client shows "Sarah moved this to Done"
rather than silently discarding one of the moves
([24](24-CONCURRENCY-AND-IDEMPOTENCY.md)).

Transitions are **not** idempotent by nature — moving to a status the task is
already in is a no-op that returns `200` without writing an event. This makes
client retries safe without an idempotency key.

## Acceptance gates

- **State enum stability test** — a compile-time exhaustive match plus a golden
  serialization fixture; adding a state fails the build until the fixture and a
  major-version ADR are updated.
- **Validation order test** — a task violating several rules at once reports the
  documented first failure.
- **Derived state invariant** — a property test over random transitions asserting
  `task.state == status.state` for every task, always.
- **Migration test** — deleting a status with 50,000 tasks migrates all of them,
  writes 50,000 activity records, and leaves no task on a deleted status.
- **Dependency gate test** — a blocked task cannot enter `ACTIVE`/`COMPLETED`;
  the override path writes an audit event with a reason.
