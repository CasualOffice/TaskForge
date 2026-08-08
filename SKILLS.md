# SKILLS.md — Domain Skills for TaskForge

These are the **competencies** required to build TaskForge — not tooling or editor
plugins. They define what an agent must understand to work responsibly in each
area. Grouped by domain.

## Working practice (per feature)

Every feature follows the same eight-step arc, mirroring
[docs/11-DESIGN-FIRST-PROCESS.md](docs/11-DESIGN-FIRST-PROCESS.md):

1. **Define the outcome** — what a user or integrator can do afterward that they
   can't now.
2. **Document the design** — a numbered `docs/` note; an ADR if a trigger fires.
3. **Compare prior art** — how Jira, Linear, GitHub, Atlassian Forge, Redmine
   handle it; record source + date checked.
4. **Identify correctness and UX risks** — wrong permission decisions, lost
   updates, unindexed queries, added concepts.
5. **Define acceptance gates** — the tests that prove it works and will keep
   working.
6. **Update the execution tracker** — a row with a stable ID and a status.
7. **Implement** in small increments.
8. **Verify** against the gates, then update docs.

## 1. Authorization & multi-tenancy

The highest-stakes domain in this repository. A wrong decision here is a security
incident, not a bug.

- **RBAC resolution** — principals, roles, grants, scope containment chains, and
  why TaskForge's model is **additive union with no deny rules**
  ([docs/04-RBAC-AND-AUTHORIZATION.md](docs/04-RBAC-AND-AUTHORIZATION.md)).
- **Why deny rules were rejected** — precedence between allow and deny is not
  predictable by the admins who configure it; Jira schemes, AWS IAM, and
  Kubernetes RBAC land on three different answers.
- **Constraint evaluation** — narrowing predicates on a grant, and the rule that
  an unconstrained grant always beats a constrained one.
- **Privilege escalation controls** — grant ceiling, scope ceiling, self-elevation
  block, last-owner protection, plugin ceiling. Each needs a test that *attempts*
  the exploit.
- **Visibility vs permission** — and why an invisible resource returns **404, not
  403**.
- **Cache correctness** — the `authz_epoch` pattern: invalidation by key
  composition rather than by fan-out, and why a cache must never authorize a
  mutation.
- **Tenant isolation as a mechanism, not a discipline** — the `WorkspaceScope`
  capability type, PostgreSQL row-level security as an independent backstop, and
  every non-database surface that also needs a tenant scope: cache keys, object
  keys, search documents, job payloads, metric labels
  ([docs/32-TENANCY-AND-ISOLATION.md](docs/32-TENANCY-AND-ISOLATION.md)).
- **Account enumeration** — login, reset, and invite responses that do not reveal
  whether an account exists.

## 2. Domain modelling & workflow

- **The status/state split** — configurable statuses over five permanent semantic
  states, and why every downstream consumer depends on it
  ([docs/23-WORKFLOW-AND-STATE-MACHINE.md](docs/23-WORKFLOW-AND-STATE-MACHINE.md)).
- **Transitions as commands** — why status is never a `PATCH`-able field.
- **Validation ordering** — running checks so the *first* reported failure is the
  most actionable one.
- **Workflow mutation under load** — deleting a status that holds tasks, changing
  a state mapping retroactively, migrating a project between workflows.
- **Dependency graphs** — `BLOCKS` semantics, cycle rejection with a depth bound,
  and gating transitions on unresolved blockers.
- **Lifecycle distinctions** — archive vs soft delete vs hard delete, grace
  periods, and GDPR anonymization-in-place versus row removal
  ([docs/03-DOMAIN-MODEL.md](docs/03-DOMAIN-MODEL.md)).

## 3. PostgreSQL & data engineering

- **Schema design** — normalized core, confined JSONB, closed enums vs `text` +
  `CHECK`, and `UNIQUE NULLS NOT DISTINCT`
  ([docs/22-DATABASE-SCHEMA.md](docs/22-DATABASE-SCHEMA.md)).
- **Index strategy as a contract** — B-tree, GIN, BRIN, partial and composite
  indexes; which query each serves; and why the filterable field set must be
  **closed** ([docs/26-SEARCH-INDEXING-AND-QUERY.md](docs/26-SEARCH-INDEXING-AND-QUERY.md)).
- **Reading `EXPLAIN (ANALYZE, BUFFERS)`** — recognising a sequential scan, a
  missing index, a bad row estimate, and a lossy bitmap heap scan.
- **Full-text search** — `tsvector`, `setweight`, `ts_rank_cd`, `pg_trgm`, and the
  reason the search document lives in a projection table rather than a generated
  column on the hot table.
- **Permission-filtered search** — filtering *before* ranking, and why
  post-filtering an authorized page breaks cursors and page counts.
- **Cursor pagination** — composite sort keys with a mandatory id tiebreaker; why
  `OFFSET` is banned.
- **Partitioning and retention** — monthly range partitions so retention is a
  partition drop, not a mass `DELETE`.
- **Row-level security** — transaction-local `set_config`, the connection-pooling
  bleed hazard, and why RLS is a backstop rather than the authorization engine.
- **Migration discipline** — forward-only, expand → migrate → contract, timing
  budgets, `CREATE INDEX CONCURRENTLY`, `NOT VALID` then `VALIDATE`.

## 4. Distributed correctness

- **The transactional outbox** — why publishing after commit is unrecoverable,
  and why the handler signature must make it impossible
  ([docs/25-EVENTS-OUTBOX-AND-AUDIT.md](docs/25-EVENTS-OUTBOX-AND-AUDIT.md)).
- **At-least-once delivery** — consumer idempotency on `event_id`; per-aggregate
  rather than global ordering; `FOR UPDATE SKIP LOCKED` for uncoordinated worker
  scaling.
- **Optimistic concurrency** — aggregate versions, `If-Match`, `409` responses
  that carry conflicting *and* safe field sets so non-overlapping edits can
  auto-merge ([docs/24-CONCURRENCY-AND-IDEMPOTENCY.md](docs/24-CONCURRENCY-AND-IDEMPOTENCY.md)).
- **Idempotency keys** — request hashing to catch key reuse, and serializing
  concurrent same-key requests.
- **Ordering under contention** — lexicographic rank strings, and why float ranks
  exhaust precision and integer ranks force renumbering.
- **Retry, backoff, dead-lettering, circuit breaking** — and the discipline that
  a dead-lettered event is never silently dropped.

## 5. Extension architecture

- **Open/closed in practice** — a closed, typed extension point registry; why
  adding a plugin must not change core code while adding a new *kind* of point
  legitimately does ([docs/34-PLUGIN-AND-EXTENSION-ARCHITECTURE.md](docs/34-PLUGIN-AND-EXTENSION-ARCHITECTURE.md)).
- **Why in-process hook systems fail** — Redmine's monkey-patching model as the
  worked negative example: plugins couple to internals, so the core stops
  refactoring.
- **Trust tiers** — declarative, remote HTTPS, managed worker, sandboxed
  frontend; choosing the least-privileged that works.
- **Scope and consent design** — least privilege, per-installation tokens,
  re-consent on escalation, the installer ceiling.
- **Failure isolation** — timeouts, circuit breakers, quotas, fail-open defaults,
  and the rule that no plugin can fail a core request.
- **Frontend sandboxing** — iframe + typed `postMessage` RPC versus ES-module
  injection, and why only the former is safe for third-party code.
- **Webhook security** — HMAC signing, replay windows, SSRF defenses, egress
  allow-lists.

## 6. Rust service engineering

- **Axum + tower** — middleware ordering, extractors, typed state, and building an
  `AuthContext` that is the *only* source of a `WorkspaceScope`.
- **SQLx** — compile-checked queries, connection pooling, transaction scoping,
  and keeping all SQL inside one crate.
- **Crate boundaries as enforcement** — expressing the layer division so an
  illegal dependency is a build failure
  ([docs/19-WORKSPACE-SCAFFOLD-DESIGN.md](docs/19-WORKSPACE-SCAFFOLD-DESIGN.md)).
- **Capability types** — newtypes whose constructors are private, so a missing
  tenant filter cannot compile.
- **Async discipline** — bounded channels, backpressure, `spawn_blocking` for CPU
  work, timeouts on every external call, and never holding a transaction across
  I/O.
- **Streaming file paths** — pre-signed direct upload, magic-byte verification,
  and never buffering an upload in RAM
  ([docs/28-ATTACHMENT-PIPELINE.md](docs/28-ATTACHMENT-PIPELINE.md)).
- **Testing topology** — pure unit tests for the resolver, property tests for
  invariants, `testcontainers` against real PostgreSQL for anything with SQL.

## 7. API & contract design

- **Versioning and compatibility** — additive-safe, removal-breaking; generated
  OpenAPI diffed in CI so the document cannot drift.
- **Error design** — stable namespaced codes, all violations returned at once, a
  `request_id` a user can quote, and messages that never leak cross-tenant
  information ([docs/20-ERROR-CODE-REGISTRY.md](docs/20-ERROR-CODE-REGISTRY.md)).
- **`PATCH` semantics** — absent versus `null`, and rejecting unknown fields.
- **SSE** — `Last-Event-ID` replay, bounded backlog, heartbeats, event coalescing,
  and re-validating membership on a long-lived stream so revocation actually
  revokes.
- **Bulk operations** — `207 Multi-Status` and why partial success is the honest
  contract when per-item rules exist.
- **Limits as a contract** — every input bounded, cheapest checks first
  ([docs/21-API-LIMITS-AND-QUOTAS.md](docs/21-API-LIMITS-AND-QUOTAS.md)).

## 8. Frontend engineering

- **Thin-client discipline** — server-side filtering, sorting, pagination, and
  authorization; the client renders ([docs/42-FRONTEND-ARCHITECTURE.md](docs/42-FRONTEND-ARCHITECTURE.md)).
- **Server-state ownership** — TanStack Query as the single cache; why a parallel
  global store becomes a divergent second copy of the truth.
- **Optimistic mutation** — rollback tokens, `409` reconciliation, and never
  discarding typed user input on failure.
- **Virtualization** — rendering only the visible window of a 2,000-card board.
- **Bundle budget discipline** — route- and feature-level code splitting, one
  library per concern, and a CI gate because bundles regress 4 KB at a time.
- **Accessibility to WCAG 2.2 AA** — keyboard-operable drag and drop, live
  regions, focus management in drawers and palettes, and knowing that automated
  checks catch only a fraction of real issues.

## 9. Operations

- **Observability that answers "why did this happen?"** — correlation IDs
  propagated through the outbox into automations, notifications, and webhooks
  ([docs/46-OBSERVABILITY-AND-OPERATIONS.md](docs/46-OBSERVABILITY-AND-OPERATIONS.md)).
- **Metric cardinality** — why `workspace_id` must not be a raw metric label.
- **Alerting on symptoms, not causes**; outbox lag as the primary health signal.
- **Degradation order** — deciding in advance what sheds first and what never
  sheds.
- **Deployment profiles** — and why one binary plus PostgreSQL being a *supported*
  profile constrains the architecture ([docs/48-DEPLOYMENT-PROFILES.md](docs/48-DEPLOYMENT-PROFILES.md)).
- **Backup and restore drills** — an untested backup is a hypothesis about a file.

## 10. Product judgement

- **The simplicity contract** — adding a capability must not add a concept; a new
  user-facing noun is an ADR trigger ([docs/17-GLOSSARY.md](docs/17-GLOSSARY.md)).
- **Progressive disclosure** — why the create form asks for a title and nothing
  else.
- **Prior-art literacy** — Jira (configurability and its cost), Linear (the feel
  bar), Atlassian Forge and GitHub Apps (extension models), Redmine (the negative
  lesson) ([docs/12-COMPETITIVE-ANALYSIS.md](docs/12-COMPETITIVE-ANALYSIS.md)).
- **Knowing what not to build** — sprints, epics, time tracking, and offline sync
  are deliberate omissions, each with a recorded reason.
