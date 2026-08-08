# 25 — Events, Outbox, Activity & Audit

Every material change produces, atomically, three records: the change itself, a
human-readable activity entry, and a dispatchable event. Plus a security-grade
audit entry where it matters. This is the mechanism behind the traceability
principle in [01](01-ORD.md), and it ships in Phase 1 (ADR-006).

## Why the outbox exists from day one

The tempting Phase 1 shortcut is to publish events after committing:

```rust
tx.commit().await?;
event_bus.publish(TaskCreated { .. }).await?;   // ← wrong
```

That line has two failure modes with no good recovery: the process dies between
commit and publish (event lost forever, no way to detect it), or the publish
fails and the change is already durable (same). Retrofitting an outbox later
means auditing every mutation path in the codebase to find the ones that got it
wrong.

So the outbox is written from the first mutation, even in Phase 1 when the only
consumer is SSE fan-out. **Eventing is never introduced later; consumers are.**

The design makes it hard to get wrong: a command handler returns
`(Change, Vec<Event>)` and `casual-task-app` commits both. A handler has no access
to a publisher, so it *cannot* emit an event outside the transaction
([19](19-WORKSPACE-SCAFFOLD-DESIGN.md) invariant 6).

## The atomic write

```
BEGIN
  UPDATE task SET ... WHERE id = $1 AND version = $2   -- the change
  INSERT INTO activity_event  (...)                    -- for people
  INSERT INTO audit_event     (...)                    -- for investigators
  INSERT INTO outbox_event    (...)                    -- for machines
COMMIT
```

There is no interleaving in which a change exists without its history. Not "we
try to keep them in sync" — the same transaction.

## Three streams, three audiences

| | **Activity** | **Audit** | **Outbox** |
| --- | --- | --- | --- |
| For | users | security & compliance | machines |
| Reads like | "Sarah moved this to Done" | actor, IP, UA, request id, before/after | typed JSON payload |
| Where | task history tab, project feed | admin console, compliance export | SSE, webhooks, workers |
| Permission | `task.history.read` | `audit.read` | scoped plugin tokens |
| Retention | project lifetime | 400 days default, configurable (ADR-025) | deleted after dispatch |
| Covers | domain changes | domain changes **+** auth, permission, plugin, admin | domain changes |

Splitting activity from audit is deliberate. Activity must be readable by anyone
on the project; audit contains IP addresses and session identifiers that most
project members should not see. One stream would force either over-exposing
audit data or under-serving the history tab.

## Event catalogue

Names are `noun.verb` in past tense, stable forever. Adding one is additive;
renaming one is a breaking change to the event schema version.

**Task** — `task.created` `task.updated` `task.status.changed` `task.closed`
`task.reopened` `task.assigned` `task.unassigned` `task.tag.added`
`task.tag.removed` `task.dependency.added` `task.moved` `task.archived`
`task.deleted`

**Collaboration** — `comment.created` `comment.updated` `comment.deleted`
`attachment.added` `attachment.committed` `attachment.deleted`

**Project & workflow** — `project.created` `project.updated`
`project.member.added` `project.member.removed` `project.archived`
`workflow.updated` `workflow.status.deleted` `milestone.completed`

**Authorization** (audit-only) — `role.created` `role.updated` `role.assigned`
`role.revoked` `permission.denied`

**Identity** (audit-only) — `auth.login` `auth.login.failed` `auth.logout`
`auth.mfa.enrolled` `token.created` `token.revoked` `user.invited`
`user.deactivated`

**Plugin** (audit-only) — `plugin.installed` `plugin.permission.granted`
`plugin.upgraded` `plugin.uninstalled` `plugin.validation.skipped`
`plugin.timeout` `plugin.circuit.opened`

`task.closed` and `task.reopened` are emitted **in addition to**
`task.status.changed`, not instead of it. Subscribers that care only about
completion should not have to interpret status-to-state mappings.

## Event envelope

```json
{
  "event_id":       "0192f8a1-...",
  "event_type":     "task.status.changed",
  "schema_version": 1,
  "workspace_id":   "...",
  "aggregate_type": "task",
  "aggregate_id":   "...",
  "project_id":     "...",
  "actor":   { "type": "USER", "id": "...", "display_name": "Sarah Johnson" },
  "occurred_at":    "2026-08-08T10:14:22.481Z",
  "request_id":     "...",
  "correlation_id": "...",
  "changes": {
    "status_id": { "from": "...", "to": "..." },
    "state":     { "from": "ACTIVE", "to": "COMPLETED" }
  },
  "metadata": { "transition_id": "...", "comment_id": null }
}
```

**`correlation_id`** is the thread that ties a user action to everything it
caused — the transition, its automation, the task that automation created, and
that task's notification all share one. It is what makes "why did this happen?"
answerable ([46](46-OBSERVABILITY-AND-OPERATIONS.md)).

**`schema_version`** is per event type. A payload change bumps it and both
versions are delivered during a deprecation window, so a plugin pinned to v1
keeps working.

**Payloads carry IDs and changed fields, never whole entities.** A subscriber
that wants the task fetches it with its own token — which means the event cannot
leak fields the subscriber is not allowed to see.

## Dispatch

A worker polls the partial index (`WHERE dispatched_at IS NULL`), which stays
small because dispatched rows leave it:

```sql
SELECT * FROM outbox_event
 WHERE dispatched_at IS NULL
 ORDER BY created_at
 LIMIT 100
   FOR UPDATE SKIP LOCKED;      -- multiple workers, no coordination
```

`FOR UPDATE SKIP LOCKED` is what allows horizontal worker scaling without a
leader election or a distributed lock.

**Polling, not `LISTEN/NOTIFY`.** Notify is fire-and-forget: a notification
delivered while no worker is connected is simply lost, so a poll loop is required
as a backstop anyway. Running one mechanism instead of two is simpler and the
100 ms poll interval is well inside every latency target. Notify may later be
added as a latency *optimization* on top, never as the delivery guarantee.

### Delivery semantics

**At-least-once.** Exactly-once across a network is not achievable, and claiming
it produces consumers that assume it. Every subscriber contract states that
consumers must be idempotent on `event_id`.

Ordering is guaranteed **per aggregate** (by `created_at`, ties broken by
UUIDv7 `event_id`), not globally. Per-aggregate order is what consumers actually
need; global order would require a single dispatcher and cap throughput.

### Retry and dead-letter

Exponential backoff — 1 s, 4 s, 16 s, 1 m, 5 m, 30 m — six attempts, then the
dead-letter queue (`attempts >= max_attempts`, its own partial index). DLQ depth
is an alerting metric, and entries are replayable from the admin console after
the cause is fixed. A dead-lettered event is never silently dropped.

### Consumer fan-out

One dispatcher, several consumers, each independently retried and independently
failing:

```
outbox_event ──▶ dispatcher ──┬──▶ SSE fan-out           (live UI)
                              ├──▶ search projection     (task_search)
                              ├──▶ notification fan-out  (per-user prefs)
                              ├──▶ automation matcher    (rule evaluation)
                              ├──▶ webhook delivery      (signed, per installation)
                              └──▶ plugin event subscribers
```

A failing webhook consumer does not delay the search projection.

## Activity records

Constructed from the same change set, rendered for humans:

```json
{
  "id": "...", "event_type": "task.status.changed",
  "actor": { "id": "...", "display_name": "Sarah Johnson", "avatar_url": "..." },
  "occurred_at": "2026-08-08T10:14:22Z",
  "changes": { "status": { "from": "In Progress", "to": "Done" } }
}
```

Note `changes` holds **status names**, not IDs — the activity stream is
rendered years later, possibly after the status has been renamed or deleted, and
it must still read correctly. Denormalizing the display value at write time is
the only way history stays truthful. The same applies to actor display names.

Activity is **append-only, enforced by grant** — the application role has no
`UPDATE` or `DELETE` on `activity_event` or `audit_event`
([22](22-DATABASE-SCHEMA.md)). Not a policy; a permission.

## Audit specifics

Audit additionally captures `ip_address`, `user_agent`, `request_id`,
`correlation_id`, `actor_type` (USER / SERVICE_ACCOUNT / PLUGIN / SYSTEM), and
`target_type`/`target_id`.

**`permission.denied` is audited.** A burst of denials is the clearest available
signal of a compromised account or a misconfigured integration, and it is
invisible if only successes are recorded.

Privacy (ADR-025): IP and user agent are retained because incident investigation
is impossible without them. Retention defaults to 400 days, is workspace
configurable within a floor of 90 days, and export is offered before a partition
is dropped. The retention policy and what is captured are documented for
end users, not buried in a config file.

## Retention and partitioning

Both event tables are monthly range-partitioned (ADR-021). A retention worker
creates next month's partition ahead of time and drops expired ones. Dropping a
partition is instant; deleting 40 million rows is hours of bloat and vacuum.

Outbox rows are deleted 7 days after dispatch — long enough to debug a delivery
problem, short enough that the table stays small.

## Acceptance gates

- **Atomicity test** — inject a failure after the `UPDATE` and before `COMMIT`;
  assert no activity, audit, or outbox row exists.
- **No-orphan-event test** — every domain mutation path produces exactly one
  outbox event; asserted by a test that enumerates command handlers.
- **Append-only test** — an `UPDATE` or `DELETE` against `activity_event` as the
  application role fails with a permission error.
- **At-least-once test** — kill the dispatcher mid-batch; on restart every event
  is delivered, some more than once, none lost.
- **Per-aggregate ordering test** — 1,000 concurrent updates to one task deliver
  in `created_at` order.
- **Correlation test** — a transition that triggers an automation that creates a
  task yields one `correlation_id` across all resulting events.
- **DLQ test** — a consumer failing 6 times dead-letters, alerts, and is
  replayable.
