# 14 — Execution Tracker

Live state of all work. Every non-trivial change gets a row before it is built
([11](11-DESIGN-FIRST-PROCESS.md)), and the row moves as it progresses.

**Last updated: 2026-08-08.**

## Status vocabulary (controlled)

| Status | Meaning |
| --- | --- |
| `Designed` | The design note is written and final. Not built. |
| `Accepted` | Design final **and** its ADRs Accepted. Ready to build. |
| `Building` | In progress. |
| `Built` | Merged, tests passing. |
| `Gated` | Built and its acceptance gates pass in CI. |
| `Blocked` | Waiting on a named dependency. |
| `Deferred` | Deliberately postponed, with a reason. |

`Gated` — not `Built` — is what "done" means. Code that passes its own tests but
has no gate protecting it will regress unnoticed.

## ID scheme

`D-###` design · `F-###` foundation (Phase 0) · `C-###` core (Phase 1) ·
`A-###` admin (Phase 2) · `P-###` platform (Phase 3) · `V-###` advanced (Phase 4).

Stable and never reused.

## Design (D)

The documentation phase. All complete unless noted.

| ID | Item | Doc | Status |
| --- | --- | --- | --- |
| D-001 | Outcome & requirements | [01](01-ORD.md) | Designed |
| D-002 | Target architecture | [02](02-ARCHITECTURE.md) | Designed |
| D-003 | Domain model & invariants | [03](03-DOMAIN-MODEL.md) | Designed |
| D-004 | **RBAC resolution algorithm** | [04](04-RBAC-AND-AUTHORIZATION.md) | Designed |
| D-005 | API & SSE contract | [05](05-API-SPEC.md) | Designed |
| D-006 | Roadmap & exit gates | [06](06-ROADMAP-AND-DELIVERY.md) | Designed |
| D-007 | Quality, security, threat model | [07](07-QUALITY-SECURITY-AND-COMPATIBILITY.md) | Designed |
| D-008 | ADR register | [08](08-ADR-REGISTER.md) | Designed |
| D-009 | Crate layer division | [19](19-WORKSPACE-SCAFFOLD-DESIGN.md) | Designed |
| D-010 | Error-code registry | [20](20-ERROR-CODE-REGISTRY.md) | Designed |
| D-011 | Limits & quotas | [21](21-API-LIMITS-AND-QUOTAS.md) | Designed |
| D-012 | Database schema | [22](22-DATABASE-SCHEMA.md) | Designed |
| D-013 | Workflow & state machine | [23](23-WORKFLOW-AND-STATE-MACHINE.md) | Designed |
| D-014 | Concurrency & idempotency | [24](24-CONCURRENCY-AND-IDEMPOTENCY.md) | Designed |
| D-015 | Events, outbox, audit | [25](25-EVENTS-OUTBOX-AND-AUDIT.md) | Designed |
| D-016 | **Search & index contract** | [26](26-SEARCH-INDEXING-AND-QUERY.md) | Designed |
| D-017 | Filter grammar & saved views | [27](27-FILTER-AND-SAVED-VIEW-DSL.md) | Designed |
| D-018 | Attachment pipeline | [28](28-ATTACHMENT-PIPELINE.md) | Designed |
| D-019 | Notifications | [29](29-NOTIFICATIONS-AND-DELIVERY.md) | Designed |
| D-020 | Performance & capacity | [30](30-PERFORMANCE-AND-CAPACITY-TARGETS.md) | Designed |
| D-021 | Tenancy & isolation | [32](32-TENANCY-AND-ISOLATION.md) | Designed |
| D-022 | **Plugin & extension architecture** | [34](34-PLUGIN-AND-EXTENSION-ARCHITECTURE.md) | Designed |
| D-023 | Automation rules | [36](36-AUTOMATION-RULES-DESIGN.md) | Designed |
| D-024 | Identity, auth, session | [40](40-IDENTITY-AUTH-AND-SESSION.md) | Designed |
| D-025 | Frontend architecture | [42](42-FRONTEND-ARCHITECTURE.md) | Designed |
| D-026 | Observability & operations | [46](46-OBSERVABILITY-AND-OPERATIONS.md) | Designed |
| D-027 | Deployment profiles | [48](48-DEPLOYMENT-PROFILES.md) | Designed |
| D-028 | Competitive analysis | [12](12-COMPETITIVE-ANALYSIS.md) | Designed |
| D-029 | Process, glossary, gates, maintenance | [11](11-DESIGN-FIRST-PROCESS.md), [15](15-CI-AND-RELEASE-GATES.md), [16](16-DOCUMENTATION-MAINTENANCE.md), [17](17-GLOSSARY.md) | Designed |
| D-030 | Support matrix | [18](18-SUPPORT-MATRIX.md) | Designed |
| D-031 | Repository & contribution | [09](09-REPOSITORY-AND-CONTRIBUTION.md), [10](10-PROJECT-GOAL-AND-STANDARDS.md) | Designed |
| D-035 | **Reporting, export & dashboards** | [38](38-REPORTING-EXPORT-AND-DASHBOARDS.md) | Designed |
| D-036 | Runbooks | [50](50-RUNBOOKS.md) | Designed |
| D-037 | Deployment guide | [52](52-DEPLOYMENT-GUIDE.md) | Designed |
| D-032 | Auth protocol ADR (session/token specifics) | [40](40-IDENTITY-AUTH-AND-SESSION.md) | **Blocked** — Accept at Phase 0 |
| D-033 | Custom-field value storage | — | **Deferred** — Accept before Phase 3 |
| D-034 | Multi-region / data residency | — | **Deferred** — no commitment until designed |
| D-038 | **Outbox dispatch: claim protocol, per-consumer state, ordering** | [25](25-EVENTS-OUTBOX-AND-AUDIT.md) | **Blocked** — Accept before C-011 |
| D-039 | Connection pool sizing, acquisition timeout, exhaustion behaviour | [30](30-PERFORMANCE-AND-CAPACITY-TARGETS.md) | **Blocked** — Accept before C-001 |
| D-040 | Queue bounds and full-queue policy | [24](24-CONCURRENCY-AND-IDEMPOTENCY.md) | **Blocked** — Accept before C-011 |
| D-041 | Cancellation and graceful shutdown | [48](48-DEPLOYMENT-PROFILES.md) | **Blocked** — Accept before C-001 |
| D-042 | Rate-limit attribution, and expiry for investigation admissions | [46](46-OBSERVABILITY-AND-OPERATIONS.md) | **Blocked** — Accept before C-001 |

Four of those are new, opened by a Phase 0 audit of the concurrency and async
design rather than by writing any code. They are recorded rather than resolved
because each is a decision, and AGENTS.md forbids settling one silently in an
implementation:

- **D-038.** [25](25-EVENTS-OUTBOX-AND-AUDIT.md) describes dispatch as holding a
  transaction open across consumer I/O, which pins a connection for the duration
  of an HTTP call to a webhook endpoint. It specifies six consumers that are
  "independently retried", but `outbox_event` carries one row-level state for all
  of them, so one consumer's failure and another's success have nowhere to be
  recorded separately. It also claims per-aggregate ordering with no mechanism
  that provides it. Three related holes, one decision.
- **D-039.** The exponential-backoff ladder in [25](25-EVENTS-OUTBOX-AND-AUDIT.md)
  is unimplementable against the committed schema: `outbox_event` (migration
  0007) has no next-attempt column, so "retry in 4 minutes" cannot be expressed
  and the dispatch poll cannot exclude a row that is waiting. This one is
  schema-visible and is cheapest to settle before the table has data in it.
- **D-040.** [24](24-CONCURRENCY-AND-IDEMPOTENCY.md) says "all queues are
  bounded" without naming a bound or what happens when one is full. The bound is
  now enforced mechanically — `clippy.toml` rejects every unbounded-channel
  constructor by resolved path — but "bounded" without a policy for the full case
  just moves the failure from an out-of-memory crash to an unspecified one.
- **D-042.** [46](46-OBSERVABILITY-AND-OPERATIONS.md) contradicts itself: §Domain
  metrics writes `rate_limit_hits_total` "by workspace", and §Cardinality
  discipline forbids a raw `workspace_id` label on any metric. The registry
  resolved it in favour of the discipline — a hashed bucket, which answers
  "is throttling concentrated?" and not "which tenant?" — and the code claimed
  that resolution was "recorded as an open question", which it was not. It is
  now. Second half of the same row: §Cardinality discipline permits per-tenant
  labels "enabled temporarily", and the allow-list enforces *small* but not
  *temporarily*. There is no clock, so an admission lasts forever. Deciding the
  expiry is a design choice, not an implementation detail.
- **D-041.** No document describes cancellation or graceful shutdown: what
  happens to an in-flight request, a half-dispatched outbox row, or a held
  advisory lock when a pod is terminated. Retrofitting cancellation through a
  codebase that never considered it is materially harder than designing it in.

## Phase 0 — Foundation (F)

| ID | Item | Status | Blocked by |
| --- | --- | --- | --- |
| F-001 | Cargo workspace + all crates with declared edges | Accepted | — |
| F-002 | Toolchain pin, MSRV ADR, `deny.toml` | Accepted | — |
| F-003 | CI: fmt, clippy, deny, nextest | Accepted | F-001 |
| F-004 | Custom architecture lints ([15](15-CI-AND-RELEASE-GATES.md)) | Accepted | F-001 |
| F-005 | PostgreSQL testcontainers harness + migration runner | Accepted | F-001 |
| F-006 | `tools/casual-task-seed` reference corpus | **Gated** | F-005 |
| F-007 | `tools/casual-task-loadtest` + baselines | `Built` | F-006 |
| F-008 | `EXPLAIN` no-seq-scan harness | **Gated** | F-006 |
| F-009 | Observability skeleton | `Built` | F-001 |
| F-010 | Docker Compose dev profile | Accepted | F-001 |
| F-011 | Governance files, Apache-2.0, AGENTS.md | Accepted | — |
| F-012 | **Bundle floor measurement** (ADR-024) | **Gated** | — |
| F-013 | Threat model review | Accepted | D-007 |
| F-014 | Runbooks (initial set) | `Built` | F-009 |
| F-015 | Migrations + application role + schema verification gate | **Gated** | F-005 |
| F-016 | Container image, deployment compose, deployment guide | **Gated** | F-015 |

## Phase 1 — Core (C)

| ID | Item | Status |
| --- | --- | --- |
| C-001 | Identity, sessions, MFA, invitations | Accepted |
| C-002 | Workspace, membership, teams | Accepted |
| C-003 | **Permission resolver + `/explain`** | Accepted |
| C-004 | Permission matrix + escalation suites | Accepted |
| C-005 | Cross-tenant property suite | Accepted |
| C-006 | Projects, membership, visibility | Accepted |
| C-007 | Default workflow + transitions | Accepted |
| C-008 | Task CRUD, assignees, tags | Accepted |
| C-009 | Comments | Accepted |
| C-010 | Attachment pipeline | Accepted |
| C-011 | Activity + audit + **outbox** | Accepted |
| C-012 | Filter grammar + compiler | Accepted |
| C-013 | Search projection + full-text | Accepted |
| C-014 | Cursor pagination | Accepted |
| C-015 | SSE + fan-out | Accepted |
| C-016 | Notifications (in-app + email) | Accepted |
| C-017 | Extension point registry (core panels only) | Accepted |
| C-018 | Web shell, board, list, My Work, drawer, palette | Accepted |
| C-019 | Bundle + a11y gates wired | Accepted |

## Phases 2–4

Rolled up until Phase 1 closes; expanded at each phase gate.

| ID | Item | Status |
| --- | --- | --- |
| A-001…A-0xx | Custom roles, simulator, workflow editor + status migration, environments, milestones, dependencies, audit console, SSO, admin console, bulk ops | Accepted (design), not scheduled |
| P-001…P-0xx | Declarative plugins, remote HTTPS, sandboxed frontend, SDK | Accepted (design), not scheduled |
| P-000 | **Three paper integrations against the extension points** | Accepted — **precondition for P-001** |
| V-001…V-0xx | Automation engine, reporting, calendar/timeline plugins, SCIM | Accepted (design), not scheduled |

## Current state

**Phase 0 — foundation. Design record complete; scaffolding under way.**

Landed and `Gated`: the Cargo workspace and dependency DAG (F-001), architecture
lints (F-004), CI (F-003), the schema — 12 migrations, the non-superuser
application role, and the verification gate proving tenant isolation and
append-only history against a real PostgreSQL 16 (F-015) — the container image
and deployment compose (F-016), and three gates added since:

- **F-006, the reference corpus.** `tools/casual-task-seed` generates the
  docs/30 workspace deterministically — 2,000,000 tasks, 38,981,941 rows, 10.2
  GiB of `COPY` text in 18.2 s at a 26 MiB peak RSS, byte-identical across runs
  and loaded into PostgreSQL 16 end to end. Gated by determinism and
  corpus-invariant tests in the `test` job.
- **F-008, the `EXPLAIN` no-seq-scan gate.** All 20 read paths planned as the
  non-superuser `taskforge_app` with RLS applied; 20 index-served, 0 sequential
  scans, 0 skips. Gated by the `explain-no-seq-scan` job.
- **F-012, the bundle floor.** Measured at 113.2 KiB gzip against ADR-024's 200
  KiB. Gated by the `bundle-size` job.

Remaining Phase 0:

- **F-007** is `Built`, not `Gated`, for one reason: the comparison gate has no
  baseline it may legitimately compare against, because the docs/30 reference
  machine does not exist. The harness, the corpus, and the gate all work and are
  tested. Recorded in [15](15-CI-AND-RELEASE-GATES.md) §Pending gates.
- **F-009** is `Built`. An audit found three defects, all now fixed and each
  covered by a test that fails without its fix: correlation fields dropped out
  of every log line emitted inside a nested span (which docs/46 §Traces makes
  the common case, so most lines would have lost them); the cardinality guard
  constrained label *keys* while leaving label *values* free, so a tenant id
  admitted for one label was accepted on another and a plugin installation id
  was accepted as a `statement` name — unbounded series on a histogram; and an
  event field named `level` silently rewrote a line's severity, which docs/46
  §Alerts fires on. It is `Built` rather than `Gated` because nothing installs
  the subscriber yet — both binaries declare the dependency and neither calls
  `init()` — and there is no CI gate on metric conformance beyond the crate's
  own tests.
- **F-014** is `Built`: the runbooks are written and cross-referenced. There is
  no meaningful CI gate on prose beyond link resolution.

None of these build product functionality — they exist to make every later phase
verifiable.

Three items are genuinely open and tracked as such: **D-032** (auth protocol
specifics, to be Accepted at Phase 0), **D-033** (custom-field storage, before
Phase 3), **D-034** (data residency, before any customer commitment).
