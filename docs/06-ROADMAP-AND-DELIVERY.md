# 06 — Roadmap & Delivery

Phases, deliverables, and exit gates. Construction is phased; **design is not**
(ADR-002). Every phase below has its architecture already written in this
docs set — the phases decide build order, not design order.

## The rule that governs the order

> A phase may not require a rewrite of anything an earlier phase built.

Four things are therefore fixed in Phase 0–1 even though their consumers arrive
much later ([02](02-ARCHITECTURE.md)):

- the **permission resolver** — Phase 1 ships built-in roles only, but through
  the final algorithm, so Phase 2 custom roles add *data*, not an engine;
- the **outbox** — written from the first mutation, when SSE is the only
  consumer;
- the **extension point registry** — core panels render through it before any
  plugin exists;
- the **index and filter contract** — defined before the first list endpoint.

## Phase 0 — Foundation

*No product functionality. This phase exists to make the following phases
verifiable.*

**Deliverables**

- Cargo workspace per [19](19-WORKSPACE-SCAFFOLD-DESIGN.md); every crate compiles
  empty with its dependency edges declared, so an illegal dependency fails the
  build from day one.
- Apache-2.0 license, `AGENTS.md`, `CONTRIBUTING.md`, `SECURITY.md`,
  `GOVERNANCE.md`, `CODE_OF_CONDUCT.md`.
- CI: fmt, clippy `-D warnings`, `cargo-deny`, `cargo-nextest`, migration test,
  bundle-size gate, OpenAPI diff gate ([15](15-CI-AND-RELEASE-GATES.md)).
- PostgreSQL 16 + `testcontainers-rs` harness; migration runner.
- `tools/casual-task-seed` producing the reference corpus deterministically
  ([30](30-PERFORMANCE-AND-CAPACITY-TARGETS.md)).
- `tools/casual-task-loadtest` + committed baselines.
- Observability skeleton: tracing, metrics, correlation IDs
  ([46](46-OBSERVABILITY-AND-OPERATIONS.md)).
- Docker Compose dev profile.
- **The schema**: migrations, the non-superuser application role, and the
  verification gate proving tenant isolation and append-only history against a
  real PostgreSQL 16 ([22](22-DATABASE-SCHEMA.md), [32](32-TENANCY-AND-ISOLATION.md)).
- **The deployable artifact**: container image, deployment compose, and the
  deployment guide ([52](52-DEPLOYMENT-GUIDE.md)) — gated end-to-end, because the
  dangerous deployment failures are silent.
- Threat model, reviewed ([07](07-QUALITY-SECURITY-AND-COMPATIBILITY.md)).
- **Bundle floor measurement** — the real React dependency set measured against
  ADR-024 before Phase 1 commits to the number.

**Exit gate**
CI green on an empty workspace · seed generates the reference corpus · the
loadtest harness runs and reports · the measured bundle floor is recorded (and
ADR-024 amended if it exceeds the budget) · the threat model is signed off.

## Phase 1 — Usable core

*A team can genuinely track work. The smallest thing that is not a toy.*

**Deliverables**

- Auth: local login, sessions, MFA, invitations ([40](40-IDENTITY-AUTH-AND-SESSION.md)).
- Workspace, membership, teams, projects, project membership.
- **The full permission resolver**, exercised by built-in roles only
  ([04](04-RBAC-AND-AUTHORIZATION.md)) — including `/permissions/explain`.
- Task CRUD, assignees, tags, comments, attachments
  ([28](28-ATTACHMENT-PIPELINE.md)).
- Default workflow + transitions ([23](23-WORKFLOW-AND-STATE-MACHINE.md)).
- Activity + audit + **outbox** ([25](25-EVENTS-OUTBOX-AND-AUDIT.md)).
- Filter grammar + built-in saved views ([27](27-FILTER-AND-SAVED-VIEW-DSL.md)).
- Search projection and full-text ([26](26-SEARCH-INDEXING-AND-QUERY.md)).
- Web client: shell, board, list, My Work, task drawer, command palette
  ([42](42-FRONTEND-ARCHITECTURE.md)).
- SSE live updates.
- Notifications: in-app + email ([29](29-NOTIFICATIONS-AND-DELIVERY.md)).
- Extension point registry, used by core panels only.

**Exit gate**
All gates in [15](15-CI-AND-RELEASE-GATES.md) · permission matrix + escalation
suites pass · `EXPLAIN` no-seq-scan suite passes at reference corpus · latency
targets met ([30](30-PERFORMANCE-AND-CAPACITY-TARGETS.md)) · bundle ≤ budget ·
WCAG 2.2 AA on core flows, keyboard-only pass done · cross-tenant property test
passes across every endpoint · backup and restore drill completed.

## Phase 2 — Administration

*A team can shape the tool to their process.*

**Deliverables**

- Custom roles and permissions; project- and environment-scoped assignments.
- **Permission simulator** UI over `/permissions/explain`.
- Custom statuses, transitions, workflow editor, and the status-migration flow
  ([23](23-WORKFLOW-AND-STATE-MACHINE.md)).
- Environments, milestones, dependencies with cycle checking.
- User-defined saved views, sharing.
- Audit console + export.
- SSO (OIDC), then SAML.
- Admin console: users, teams, sessions, tokens, quotas.
- Bulk operations, async job endpoint.

**Exit gate**
Custom roles pass the same matrix suite as built-ins · workflow migration moves
50,000 tasks with complete history · SSO tested against a real IdP in CI · audit
export verified against a compliance checklist.

## Phase 3 — Extension platform

*Others can build on it without forking it.*

- **3a** Declarative plugins: manifest, validation, consent, audit, custom fields,
  declarative automations.
- **3b** Remote HTTPS: webhooks, signing, task actions, `validation.transition`,
  notification channels, per-installation observability.
- **3c** Sandboxed frontend: iframe panels, project tabs, command registration,
  settings sections.
- Integration SDK + plugin developer documentation.

**Exit gate**
Three real plugins built against the contract by someone who did not design it ·
plugin failure isolation proven (a plugin that hangs, errors, and floods leaves
core latency unchanged) · scope escalation rejected · uninstall grace period
verified.

> **Precondition for 3a:** three real integrations designed on paper against the
> extension points *before* implementation begins. If they need points that do
> not exist, the registry is wrong and it is cheap to fix now
> ([12](12-COMPETITIVE-ANALYSIS.md) §risks).

## Phase 4 — Advanced productivity

- Full automation rules engine with the builder and dry-run
  ([36](36-AUTOMATION-RULES-DESIGN.md)).
- Reporting projections; cycle time, throughput, burndown.
- Calendar and timeline as first-party plugins — proving the plugin path carries
  real features, not just toys.
- Enterprise SSO controls: SCIM provisioning, session policy.
- Optional external search, only if ADR-014's tripwire fired.

**Exit gate**
Automation loop and escalation tests pass · reports agree with a
hand-calculated fixture · calendar/timeline ship *as plugins* with no core change.

## Cross-phase, always

Applies to every phase; not a phase of its own:

- Migrations are expand → migrate → contract ([22](22-DATABASE-SCHEMA.md)).
- Every behaviour change ships with tests.
- Every design change updates its numbered doc and the tracker
  ([16](16-DOCUMENTATION-MAINTENANCE.md)).
- Backup/restore drills each phase — a backup never restored is not a backup.
- Dependency and container scanning on every build.

## Sequencing risks

| Risk | Mitigation |
| --- | --- |
| Phase 1 grows to absorb Phase 2 | The exit gate is a checklist, not a judgement call. Custom roles are explicitly out of Phase 1. |
| The bundle budget is unreachable | Measured in Phase 0, before it is promised (ADR-024). |
| Extension points are wrong | Three paper integrations before 3a. |
| Search doesn't hold at scale | Reference corpus and `EXPLAIN` gates exist from Phase 0, so the truth arrives early. |
| The permission model is too rigid | The additive trade is documented ([04](04-RBAC-AND-AUTHORIZATION.md)); revisiting it requires a superseding ADR, not a patch under deadline. |
