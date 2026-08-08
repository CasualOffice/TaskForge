# 02 — Architecture

TaskForge is a **Rust modular monolith with an extension plane.** One deployable
API binary owns the domain; worker binaries own everything asynchronous; plugins
run outside both. This doc is the shape; the crate-level layer division is
[19](19-WORKSPACE-SCAFFOLD-DESIGN.md).

## The stack

```
                 ┌────────────────────────────────────────────────┐
   Browser ─────▶│  Web client (React + TS, thin)                  │  webapp/
                 └────────────────────────────────────────────────┘
                            │ REST /api/v1  +  SSE /api/v1/stream
                 ┌──────────┴─────────────────────────────────────┐
                 │  Edge:  tower middleware                        │  casual-task-api
                 │   auth ctx · authz · request id · limits ·      │
                 │   timeouts · compression · tracing              │
                 ├────────────────────────────────────────────────┤
                 │  Application: command & query handlers          │  casual-task-app
                 │   one transaction per command                   │
                 ├────────────────────────────────────────────────┤
                 │  Domain modules (compile-time boundaries)       │
                 │   identity · workspace · project · workflow ·   │  casual-task-*
                 │   task · activity · attachment · notification   │
                 │            ▲                                    │
                 │      casual-task-authz  (consulted by all)      │
                 ├────────────────────────────────────────────────┤
                 │  Persistence: repositories over SQLx            │  casual-task-persistence
                 └──────────┬─────────────────────────────────────┘
                            │  one PostgreSQL transaction
                 ┌──────────┴─────────────────────────────────────┐
                 │  PostgreSQL — system of record + outbox + index │
                 └──────────┬─────────────────────────────────────┘
                            │  outbox poll
                 ┌──────────┴─────────────────────────────────────┐
                 │  Workers                                        │  casual-task-worker
                 │   dispatch · notify · webhook · scan · search   │
                 │   projection · automation · plugin jobs         │
                 └──────────┬─────────────────────────────────────┘
                            │  scoped, signed, timed, quota'd
                 ┌──────────┴─────────────────────────────────────┐
                 │  Extension plane (never in-process)             │
                 │   declarative · remote HTTPS · managed worker · │
                 │   sandboxed frontend module                     │
                 └────────────────────────────────────────────────┘
```

Optional infrastructure — **Redis** (rate limits, SSE fan-out, short caches) and
**S3-compatible object storage** (attachments) — is genuinely optional: the
single-node profile runs without either ([48](48-DEPLOYMENT-PROFILES.md)).

## Core principles

1. **The server is the authority.** Every mutation is authorized server-side
   against the actor's effective permissions. The client renders affordances from
   a permission set; it never *decides* anything ([04](04-RBAC-AND-AUTHORIZATION.md)).
2. **One command, one transaction, one history record.** A domain mutation, its
   activity record, and its outbox event commit together or not at all. There is
   no window in which a change exists without its history
   ([25](25-EVENTS-OUTBOX-AND-AUDIT.md)).
3. **Status is configurable; state is not.** Five semantic states
   (`BACKLOG` `PLANNED` `ACTIVE` `COMPLETED` `CANCELED`) are the permanent
   contract for APIs, reports, and plugins. Teams rename and rewire statuses
   freely above them ([23](23-WORKFLOW-AND-STATE-MACHINE.md)).
4. **Transitions are commands, not field writes.** You cannot `PATCH` a status.
   `POST /tasks/{id}/transitions` is the only door, so validation, permissions,
   required fields, and automations cannot be bypassed.
5. **Modules are compile-time boundaries.** A crate never reads another module's
   tables. Cross-module reads go through an application interface; cross-module
   reactions go through domain events.
6. **Every tenant access carries a workspace scope.** Enforced by types, not
   discipline: repositories take a `WorkspaceScope` that only the auth middleware
   can mint ([32](32-TENANCY-AND-ISOLATION.md)).
7. **Open for extension, closed for modification.** Adding a plugin never touches
   core code. The set of *extension points* is a versioned registry; adding a new
   kind of extension point requires an ADR
   ([34](34-PLUGIN-AND-EXTENSION-ARCHITECTURE.md)).
8. **No unindexed user-reachable query.** Filterable and sortable fields are a
   closed set, and each has a named index that serves it
   ([26](26-SEARCH-INDEXING-AND-QUERY.md)).
9. **The client is a thin interaction layer.** Filtering, sorting, pagination,
   permission evaluation, and workflow validation are server concerns
   ([42](42-FRONTEND-ARCHITECTURE.md)).
10. **Nothing external is on the critical path.** Plugin, webhook, notification,
    scan, and search-projection work happens after commit, in workers, with
    timeouts and backpressure. A slow integration cannot slow a board.

## Why a modular monolith

A tracker's aggregates are small and its cross-entity invariants are dense: a
transition touches task, workflow, permission, dependency, activity, and outbox
at once. Distributing that across services buys nothing and costs a distributed
transaction. Rust crates give the module boundaries; the process boundary is not
needed to enforce them, and it is not free.

**What "modular" buys us concretely:** the crate DAG makes an illegal dependency
a *compile error*, not a review comment. If `casual-task-task` ever needs to read
a `project_membership` row directly, the build fails.

**When a module may leave the monolith:** only when it has an independent scaling
profile *and* an eventually-consistent boundary — which today means workers
(already separate binaries) and nothing else. Splitting anything further requires
an ADR ([08](08-ADR-REGISTER.md)).

## Why Rust (ADR-001)

Not for raw throughput — a tracker is I/O-bound. The reasons that actually apply:

- **Predictable memory and startup** — self-hosters run this on small boxes; a
  200 MB idle floor is a real cost to them.
- **Compile-time module boundaries** — `cargo` enforces the layer division that a
  package-by-convention layout only suggests.
- **Compile-checked SQL** — `sqlx::query!` verifies every statement against the
  real schema at build time, which is exactly the discipline a 30-table
  permission-sensitive schema needs.
- **Streaming file paths without ceremony** — attachments never land in RAM
  ([28](28-ATTACHMENT-PIPELINE.md)).
- **Suite consistency** — OpenDoc and OpenCalc are Rust; shared toolchain, shared
  CI shape, shared people.

This does **not** license unsafe code or premature optimization: `unsafe_code`
is `forbid` at the workspace root, and any exception needs an ADR.

## Independent version numbers

Four things version separately so one can move without falsely implying the
others did:

- **API version** (`/api/v1`) — the REST/SSE surface ([05](05-API-SPEC.md)).
- **Schema version** — migrations ([22](22-DATABASE-SCHEMA.md)).
- **Event schema version** — the outbox/webhook payload contract
  ([25](25-EVENTS-OUTBOX-AND-AUDIT.md)).
- **Plugin contract version** — extension points and scopes
  ([34](34-PLUGIN-AND-EXTENSION-ARCHITECTURE.md)).

A plugin declares a compatibility range against the *plugin contract* version
only. It is never coupled to the schema.

## Request lifecycle: a status transition

The path that exercises nearly every rule:

```
POST /api/v1/tasks/{id}/transitions   { to_status, fields?, comment? }
   │
   ├─ edge:  authenticate → AuthContext{ actor, workspace }        casual-task-api
   ├─ edge:  rate limit, request id, timeout, trace span
   │
   ├─ app:   load task within WorkspaceScope                       casual-task-app
   ├─ authz: effective_permissions(actor, project) ∋ task.transition
   │           …and the transition's own required permission        casual-task-authz
   ├─ domain: workflow validates edge exists                       casual-task-workflow
   ├─ domain: required fields present for target status
   ├─ domain: blocking dependencies resolved                       casual-task-task
   │
   ├─ BEGIN
   │    UPDATE task ... WHERE id = $1 AND version = $2   ← optimistic concurrency
   │    INSERT activity_event  (human-readable)
   │    INSERT audit_event     (security-grade)
   │    INSERT outbox_event    (task.status.changed)
   │  COMMIT                                              ← 409 here if version stale
   │
   └─ 200 + new representation (+ ETag: version)

   later, out of band:
     worker → SSE fan-out          → subscribers revalidate membership
     worker → search projection    → task_search row refreshed
     worker → automation rules     → matching rules enqueued
     worker → notifications        → per-user preference fan-out
     worker → webhooks / plugins   → signed, timed, retried, dead-lettered
```

Everything after `COMMIT` is asynchronous and failure-isolated. If the plugin
plane is entirely down, the transition still succeeds.

## Security boundary

- Workspace scope is a type, minted only by authenticated middleware, required by
  every repository call ([32](32-TENANCY-AND-ISOLATION.md)).
- Defense in depth: PostgreSQL row-level security is enabled on tenant tables as
  a backstop behind the type-level guarantee, not instead of it.
- Plugins get scoped tokens, never database access; every call is timed, quota'd,
  circuit-broken, and egress-restricted.
- Uploads stream to object storage and stay invisible until scanned and committed.
- No customer code runs in the API process. Ever.

Full posture: [07](07-QUALITY-SECURITY-AND-COMPATIBILITY.md).

## Why this shape survives the phased build

The risk in a phased service is that a later phase forces an earlier rewrite.
Four things are fixed now to prevent that:

- **The permission resolution algorithm** ([04](04-RBAC-AND-AUTHORIZATION.md)) —
  Phase 1 ships simple built-in roles, but they are evaluated by the *final*
  resolver, so Phase 2 custom roles add data, not a new engine.
- **The outbox** ([25](25-EVENTS-OUTBOX-AND-AUDIT.md)) — written from the first
  mutation in Phase 1, even when the only consumer is SSE. Phases 3–4 attach
  consumers; they do not introduce eventing.
- **The extension point registry** ([34](34-PLUGIN-AND-EXTENSION-ARCHITECTURE.md))
  — Phase 1 renders core panels *through* the same registry plugins will use, so
  the seam is proven before any plugin exists.
- **The index and filter contract** ([26](26-SEARCH-INDEXING-AND-QUERY.md),
  [27](27-FILTER-AND-SAVED-VIEW-DSL.md)) — the closed field set and its indexes
  are defined before the first list endpoint, so saved views and search never
  need a schema redesign.

These are ADR-004, ADR-006, ADR-009, and ADR-011 in [08](08-ADR-REGISTER.md).
