# 24 — Concurrency, Conflicts & Idempotency

How simultaneous edits, retried requests, and racing workers are made safe.
A tracker is a multi-user, optimistically-updated, unreliable-network application;
these are the mechanics that keep it honest.

## Optimistic concurrency (ADR-023)

Every mutable aggregate carries `version bigint`, incremented on every write.

```sql
UPDATE task
   SET title = $1, version = version + 1, updated_at = now(), updated_by = $2
 WHERE id = $3 AND version = $4;
-- 0 rows affected ⇒ someone else wrote first ⇒ 409
```

Exposed as an `ETag`; required as `If-Match` ([05](05-API-SPEC.md)).

**Why not last-write-wins.** Silent overwrite is the most-reported bug class in
collaborative tools, and it is unfalsifiable from the user's side: the edit
simply vanishes with no error and no trace. The cost of optimistic concurrency is
an occasional `409` the client must handle. That is a real cost, paid once in the
client, versus a permanent invisible data-loss channel.

**Why not pessimistic locking.** Row locks held across a user's think-time
require lock expiry, lock stealing, and a UI for "Sarah is editing this" — a
large amount of machinery for a conflict rate that is, in practice, low.

**Why not CRDTs.** Right for character-level co-editing of a document (that is
OpenDoc's problem). A task is a record of typed fields, not a text buffer; field-
level conflict detection is what users understand and what audit needs.

### The conflict response

A `409` that just says "conflict" pushes the whole problem to the user. Ours
carries what is needed to resolve it:

```json
{
  "error": {
    "code": "TF-CNC-0001",
    "message": "This task was updated by someone else",
    "details": {
      "your_version": 7,
      "current_version": 9,
      "conflicting_fields": ["status_id", "assignees"],
      "your_safe_fields": ["title"],
      "changed_by": { "id": "...", "display_name": "Sarah Johnson" },
      "changed_at": "2026-08-08T10:14:22Z"
    },
    "current": { "...full current representation..." }
  }
}
```

`conflicting_fields` vs `your_safe_fields` is the important part: if the fields
you changed do not overlap with the fields they changed, the client can retry
automatically against the new version without asking anyone. Most conflicts in
practice are non-overlapping (you edited the description, they moved the status),
and resolving those silently is the difference between a system that feels
collaborative and one that nags.

Overlapping conflicts surface a diff and let the user choose. The client never
picks for them.

## Optimistic UI and rollback

The client applies mutations locally before the server responds
([42](42-FRONTEND-ARCHITECTURE.md)):

```
user drags card
  → apply locally, immediately        (optimistic)
  → POST transition with If-Match
      → 200  : reconcile with server representation
      → 409  : non-overlapping? retry once against the new version
               overlapping?     roll back, show what changed, keep the user's input
      → 5xx  : roll back, queue for retry, show offline indicator
```

**Rollback preserves user input.** A failed comment must not clear the textarea —
the text goes back into the draft cache. Losing typed text on a network blip is
the fastest way to lose a user's trust.

## Idempotency for creates

`POST` without protection is unsafe on retry: a timeout that actually succeeded
produces a duplicate task, and the user has no way to tell.

```http
POST /api/v1/projects/{id}/tasks
Idempotency-Key: 018f2c9e-...        ← client-generated UUIDv7, required
```

```
BEGIN
  INSERT INTO idempotency_key (workspace_id, actor_id, key, request_hash)
  VALUES (...)
  ON CONFLICT DO NOTHING;
  -- 0 rows ⇒ this key was seen before:
  --    same request_hash → return the stored response
  --    different hash    → 422 TF-IDM-0002 (key reused with a different body)
  ... perform the create ...
  UPDATE idempotency_key SET response = $1, status_code = 201 WHERE ...;
COMMIT
```

- Scoped to `(workspace, actor, key)` — one client's key cannot collide with
  another's.
- `request_hash` catches the common client bug of generating a key once and
  reusing it for a different task. Without it, the second task silently returns
  the first task's response and the user thinks it was created.
- Retained 24 hours, then swept.
- **A concurrent in-flight request with the same key gets `409 TF-IDM-0001`**
  ("in progress, retry"), not a duplicate. The `ON CONFLICT DO NOTHING` insert
  inside the transaction serializes them.

`PATCH` and `DELETE` are naturally idempotent under `If-Match` — a retry with a
stale version returns `409`, which is the correct answer.

## Task number allocation (ADR-008)

```sql
UPDATE project SET task_seq = task_seq + 1 WHERE id = $1 RETURNING task_seq;
```

A row lock inside the creating transaction. Concurrent creates in one project
serialize on it; concurrent creates in *different* projects do not contend at all.

**Why not a sequence.** Sequences are non-transactional: a rollback consumes the
number permanently, producing `WR-1, WR-2, WR-4`. Users read gaps as lost data and
file support tickets about it. The lock is held for microseconds and bounded by
per-project creation rate, which is never the bottleneck.

## Board reordering

The highest-contention operation: several people dragging cards in one column.

Lexicographic ranks (ADR-013) make each drag a single-row update — a card moving
between neighbours `a0` and `a1` computes `a0V` and writes only itself. Two
concurrent drags to the same slot both succeed and produce a deterministic order
by `(position, id)`; nobody's drag is rejected. There is no column-wide lock and
no renumbering.

Rank collision (identical strings) is possible but harmless — `id` breaks the tie
deterministically, and a compaction job rewrites collided ranks later.

## Dependency cycle checking

Adding `A blocks B` requires proving no path `B → … → A` exists. Two racing
requests could each individually be acyclic while jointly creating a cycle.

Fixed by taking a **workspace-level advisory lock** on the dependency graph for
the duration of the check-and-insert:

```sql
SELECT pg_advisory_xact_lock(hashtext('dep:' || $workspace_id));
-- reachability check, depth-limited to 64
INSERT INTO task_dependency ...;
```

Coarse, and deliberately so: dependency edits are rare (orders of magnitude rarer
than task updates), and a correct coarse lock beats a subtle fine-grained scheme
for an operation nobody performs in a hot loop. The lock is transaction-scoped, so
it releases automatically.

## Worker concurrency

- **Outbox dispatch** — `FOR UPDATE SKIP LOCKED` lets N workers share the queue
  without coordination ([25](25-EVENTS-OUTBOX-AND-AUDIT.md)).
- **Scheduled jobs** (retention, compaction, reminders) — a leader lease in
  PostgreSQL (`pg_advisory_lock` + heartbeat), so exactly one instance runs them.
- **Consumers are idempotent on `event_id`.** Delivery is at-least-once; a
  consumer that assumes exactly-once is a bug waiting for a redeploy.
- **All queues are bounded**, with backpressure. An unbounded channel is a
  deferred out-of-memory crash. `clippy.toml` rejects every unbounded-channel
  constructor by resolved path, so this is a build failure rather than a rule.

### Every bound names its overflow policy (D-040)

"Bounded" without a policy for the full case moves the failure from an
out-of-memory crash to an unspecified one. Each bound below states what happens
when it is reached; a new bound without that line is incomplete.

| Bound | Value | When it is reached |
| --- | --- | --- |
| Outbox claim batch | 64 deliveries per poll | The rest stay in the database — **the database is the queue**. There is no in-memory backlog to lose, and a worker restart costs a poll, not a batch. |
| Outbox deliveries in flight | 16 per consumer loop | The loop **waits for a permit before claiming more**, which stops claiming and lets `next_attempt_at` and the claim expiry do the rest. Spawning instead is how a slow consumer becomes thousands of sockets. |
| Delivery attempts | 6, then the dead-letter queue | The delivery is dead-lettered and **kept**, never dropped ([25](25-EVENTS-OUTBOX-AND-AUDIT.md)); RB-02 in [50](50-RUNBOOKS.md) is the recovery path. |

The permit is acquired **before** the delivery task is spawned. Acquiring inside
the task would bound the concurrent *work* while leaving the number of tasks
unbounded, which is the same memory problem wearing a semaphore.

### Cancellation and graceful shutdown (D-041)

On `SIGTERM` a worker **stops claiming first, then drains**, bounded by a
deadline shorter than the orchestrator's kill grace. The order is the
load-bearing part: a drain that kept claiming would never finish under load, and
the process would be `SIGKILL`ed mid-delivery instead — which is strictly worse
than a clean abandon.

- Every sleep in the loop is cancellable. A worker asked to stop must not first
  sit out its poll interval.
- Deliveries still running when the drain expires are **abandoned, not awaited**.
  Their rows stay claimed and become claimable again after the claim expiry, so
  the work is delayed rather than lost. The duplicate delivery that follows is
  expected under at-least-once, not an incident.
- The drain deadline is shorter than the claim expiry. Otherwise a drain could
  still be running when another worker becomes entitled to the same rows,
  turning every shutdown into a guaranteed double delivery rather than a rare
  one.
- Database transactions need no special handling: a dropped connection rolls
  back. That is why the claim commits before delivery begins — there is no
  transaction open to lose.

## Transaction discipline

1. **One command, one transaction.** No handler opens two.
2. **No I/O inside a transaction** — no HTTP, no object store, no plugin call.
   A transaction holding a lock while awaiting a slow third party is how a
   database gets exhausted. Everything external happens before (validation) or
   after (via outbox).
3. **`READ COMMITTED`** is the default; `SERIALIZABLE` only where a genuine
   write-skew exists, with a retry loop. Blanket `SERIALIZABLE` trades a rare
   correctness bug for a constant stream of serialization failures.
4. **Statement timeout 5 s**; transaction timeout 10 s. A runaway transaction is
   killed, not tolerated.
5. **Connection pool sized by cores, not by hope** — an oversized pool converts a
   database slowdown into a full outage by queueing every request onto a saturated
   server.

## Acceptance gates

- **Lost-update test** — two concurrent `PATCH`es with the same `If-Match`; exactly
  one succeeds, the other gets `409` with the correct conflicting-field set.
- **Auto-merge test** — non-overlapping concurrent edits reconcile without user
  input; overlapping ones do not.
- **Idempotency race** — 50 concurrent identical creates with one key produce
  exactly one task.
- **Key-reuse test** — same key, different body ⇒ `422`, and no second task.
- **Number-gap test** — 1,000 creates with 30% induced rollbacks produce a
  contiguous number sequence.
- **Rank test** — 10,000 concurrent reorders leave a total order with no
  duplicates past the compaction threshold.
- **Cycle race** — concurrent `A→B` and `B→A` inserts: exactly one succeeds.
- **No-I/O-in-transaction lint** — a compile-time check that no HTTP or object
  store client type is reachable from a transaction scope.
