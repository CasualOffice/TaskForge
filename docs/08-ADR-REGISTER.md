# 08 — ADR Register

Architecture Decision Records capture decisions that are expensive to reverse —
the ones listed under "ADR triggers" in [11](11-DESIGN-FIRST-PROCESS.md). Each has
a stable number, a status, and a short rationale. ADRs are **append-only**: a
superseded decision is marked `Superseded by ADR-NNN`, never edited away.

This register closes the gap the old drafts left open. Their §18 listed nine
"decisions required before coding" and answered none of them. Every one is
resolved below, with the reasoning in the linked design note.

## Status values

`Proposed` · `Accepted` · `Superseded` · `Rejected`

## Register

| ADR | Title | Status | Summary |
| --- | --- | --- | --- |
| **ADR-001** | Rust/Axum/SQLx; product name TaskForge, crates `casual-task-*` | Accepted | Rust stable + Tokio + Axum + tower + SQLx + PostgreSQL. Chosen for predictable memory, compile-time module boundaries, compile-checked SQL, streaming file paths, and suite consistency with OpenDoc/OpenCalc — not for raw throughput. Supersedes the archived Java/Spring drafts. See [02](02-ARCHITECTURE.md). |
| **ADR-002** | Design-first, phased delivery | Accepted | The full architecture is designed up front; construction is phased. A design that would force an earlier layer to be rewritten is rejected. See [06](06-ROADMAP-AND-DELIVERY.md), [11](11-DESIGN-FIRST-PROCESS.md). |
| **ADR-003** | Modular monolith; layer division fixed before code | Accepted | One API binary + worker binaries. The crate set and dependency DAG in [19](19-WORKSPACE-SCAFFOLD-DESIGN.md) are the contract; changing a boundary requires a new ADR. Splitting a module into a service requires an ADR and an independent scaling justification. |
| **ADR-004** | Additive-union RBAC, no deny rules | Accepted | Effective permissions are the union of all grants whose scope contains the resource. There are no deny grants and no precedence rules. Cost accepted: "everyone except X" is expressed by removing a grant or by project visibility. See [04](04-RBAC-AND-AUTHORIZATION.md). |
| **ADR-005** | `TASK` scope excluded from v1 | Accepted | Grants apply at WORKSPACE/TEAM/PROJECT/ENVIRONMENT only. Per-task grants make the grant table scale with task count and the resolver unbounded, and they break the single-resolution-per-list optimization. Exceptional sharing is deferred to a token-based share link. |
| **ADR-006** | Transactional outbox from the first mutation | Accepted | Domain change + activity + audit + outbox commit in one transaction, from Phase 1, even when SSE is the only consumer. Eventing is never introduced later. See [25](25-EVENTS-OUTBOX-AND-AUDIT.md). |
| **ADR-007** | Project keys are immutable | Accepted | Task keys (`WR-125`) appear in commits, chat, and external tickets. A rename would invalidate every external reference. Renaming a project does not change its key. See [03](03-DOMAIN-MODEL.md). |
| **ADR-008** | Task numbers allocated in-transaction, not by sequence | Accepted | `UPDATE project SET task_seq = task_seq + 1 RETURNING task_seq` inside the creating transaction. Sequences leak numbers on rollback and users read gaps as data loss. Contention is bounded by per-project creation rate. |
| **ADR-009** | Closed, typed extension point registry; core uses it too | Accepted | Adding a plugin never changes core code. Adding a new *kind* of extension point does, and requires an ADR. Core task panels render through the same registry, so the seam is proven in Phase 1. See [34](34-PLUGIN-AND-EXTENSION-ARCHITECTURE.md). |
| **ADR-010** | Multiple assignees with optional primary; single-select environment | Accepted | Answers two open questions from the old §18. Multi-assignee via `task_assignee` from day one (single-assignee breaks pairing, review, and incident cases). Environment single-select — multi doubles filter/report surface for a case better modelled as linked tasks; single→multi is additive later. |
| **ADR-011** | Closed filterable/sortable field set, one named index each | Accepted | A filter or sort on an unlisted field is a `400`, not a slow query. This is what makes "no sequential scans" enforceable rather than aspirational. See [26](26-SEARCH-INDEXING-AND-QUERY.md). |
| **ADR-012** | `authz_epoch` cache invalidation | Accepted | Per-workspace counter bumped in the same transaction as any grant/membership change; it is part of the cache key, so stale entries simply miss. No invalidation fan-out. The cache never authorizes a mutation. |
| **ADR-013** | Lexicographic rank strings for manual ordering | Accepted | Float ranks exhaust precision after ~50 drags between neighbours; integer ranks need column renumbering on every insert. Lexicographic ranks insert between any pair, sort with a plain B-tree, and are compacted by a background job. |
| **ADR-014** | PostgreSQL-native search, with a measured tripwire | Accepted | `tsvector` + GIN + `pg_trgm` over a projection table. Transactionally consistent with permissions, which an external index cannot be. An external engine is introduced only on measured criteria (p95 > 300 ms at reference corpus, a genuinely unservable requirement, or >20% write overhead). |
| **ADR-015** | Plugin contract versioned independently of app and schema | Accepted | Plugins pin a contract range, never an app version or schema version. This is what allows weekly releases without ecosystem breakage. |
| **ADR-016** | Managed-worker plugins deferred past v1 | Accepted | Running customer containers triples the operational surface (image supply chain, sandbox escape, noisy neighbours, cost attribution) for a capability the declarative and remote classes cover for nearly every real integration. The manifest reserves the slot so adding it is additive. |
| **ADR-017** | `validation.transition` fails open by default | Accepted | The one synchronous plugin point is bounded at 500 ms with no retry and allows the transition on timeout or error. A workspace admin may opt a specific plugin into fail-closed, explicitly. A broken integration must not stop a team from working. |
| **ADR-018** | Subtask depth capped at 1; no automatic status rollup | Accepted | Arbitrary trees force recursive queries into every list, board, and permission check, and users build unnavigable hierarchies. Parent status is displayed as a rollup, never derived — implicit status changes are the most confusing behaviour in trackers that do it. |
| **ADR-019** | `BLOCKS`-only dependencies; cycles rejected; transitions gated | Accepted | One dependency kind in v1. Cycles rejected at write time by a depth-limited reachability check. Unresolved blockers prevent entry to `ACTIVE`/`COMPLETED` unless the transition opts out or the actor holds an audited override. |
| **ADR-020** | PostgreSQL 16; RLS as a tenancy backstop | Accepted | Tenancy is enforced by a `WorkspaceScope` type only auth middleware can mint. RLS is enabled behind it as defense in depth — two independent mechanisms must both fail to leak across tenants. RLS is *not* the authorization engine. See [32](32-TENANCY-AND-ISOLATION.md). |
| **ADR-021** | Monthly range partitioning for activity and audit | Accepted | Retention becomes `DROP TABLE partition` rather than a mass `DELETE` that bloats and vacuums for hours. Append-only is enforced by revoking UPDATE/DELETE from the application role, not by convention. |
| **ADR-022** | Confined JSONB policy | Accepted | JSONB in exactly five places, each with a validated schema and written justification. Anything filterable or indexable gets a typed table — including plugin custom-field values. |
| **ADR-023** | Optimistic concurrency via aggregate `version` + `If-Match` | Accepted | Conflicts return `409` with the current representation and the changed field set, so the client can merge rather than clobber. Last-write-wins is rejected: silent overwrite is the most-reported bug class in collaborative editors. See [24](24-CONCURRENCY-AND-IDEMPOTENCY.md). |
| **ADR-024** | Bundle budget 200 KiB, measured before it is promised | Accepted | **Measured in Phase 0 and the number held**: the real dependency floor is 113.2 KiB gzip against a 200 KiB budget, so no superseding ADR was needed ([webapp/BUNDLE-FLOOR.md](../webapp/BUNDLE-FLOOR.md), F-012). The measurement fixed three things the original number left open: the unit is **KiB (204,800 bytes), gzip, initial chunk only** — the ambiguity was worth 4.7 KiB, more than one whole dependency; "initial" means the entry chunk plus its static-import closure plus imported CSS, excluding `import()`ed chunks; and the floor moves 4.4% on a bundler major with no source change, so the gate freezes the lockfile. Enforced by the `bundle-size` job. |
| **ADR-025** | Audit retention 400 days default; IP/UA stored, per-workspace policy | Accepted | Answers the last open §18 question. Audit retains IP and user agent because incident investigation is impossible without them; retention is workspace-configurable within a policy floor, and export is available before partition drop. See [07](07-QUALITY-SECURITY-AND-COMPATIBILITY.md). |
| **ADR-026** | User deletion anonymizes in place | Accepted | A deleted user becomes a tombstone; authored tasks, comments, and history keep their foreign keys. Deleting history to erase a person would destroy the audit trail for everyone else. |
| **ADR-027** | Reports are saved filters plus a closed measure set | Accepted | A report is a filter ([27](27-FILTER-AND-SAVED-VIEW-DSL.md)) plus an aggregation over the same closed field set. No user-defined SQL, no BI query builder, no calculated fields — those would break the no-sequential-scan promise (ADR-011) exactly where load is highest. Real BI needs are served by export. See [38](38-REPORTING-EXPORT-AND-DASHBOARDS.md). |
| **ADR-028** | `task_state_interval` projection as the metric substrate | Accepted | Cycle time, lead time, and time-in-state are derived from state history, not from a `resolved_at` column. Computing them by scanning `activity_event` at query time is the unbounded query reporting must not introduce. The projection is maintained by the outbox worker and is fully rebuildable from `activity_event`, so it is a cache and not a second source of truth. |
| **ADR-029** | Export is async above 1,000 rows, per-batch authorized, always audited | Accepted | Exports stream to object storage from the worker; the API never holds a result set. Permissions are re-evaluated per batch so an actor who loses access mid-export stops receiving rows. Every export writes an audit event — bulk data leaving the system is precisely what an audit trail is for. Cells beginning `=`, `+`, `-`, or `@` are quote-prefixed against CSV formula injection. |
| **ADR-030** | XLSX export via OpenCalc | Accepted | The suite already maintains a Rust `.xlsx` engine, so spreadsheet export costs a dependency edge rather than a third-party writer and a new supply-chain surface. |
| **ADR-031** | MSRV is the floor the dependency tree forces, not a chosen number | Accepted | MSRV = the lowest stable rustc the workspace and its locked dependencies actually build and test on; today that is **1.88.0**, forced by `time` 0.3.55. Raising it requires the PR to name the crate and version that forced it. The dev toolchain (`rust-toolchain.toml`) tracks current stable and is a separate number; CI tests both ends. Accepted at Phase 0 (D-044); `rust-version` and the CI matrix now read the measured floor. |
| **ADR-032** | Auth mechanism: selector/verifier credentials, uncached sessions, a narrowed pre-workspace seam | Accepted | Settles the layer beneath [40](40-IDENTITY-AUTH-AND-SESSION.md) where the record and the `Gated` schema contradicted each other. Credentials are **selector/verifier** — an indexed non-secret selector plus a per-row-salted verifier hash — so authentication stays one indexed read and **no server-held pepper is load-bearing**; a keyed-HMAC alternative was rejected for the key-custody burden it created. Sessions and tokens are **never cached**: the staleness window is the stated reason JWTs were rejected. `principal_type` is **not** extended — a plugin installation authenticates but is not something a role is assigned to, so [04](04-RBAC-AND-AUTHORIZATION.md) stays the only authority model. The pre-workspace lookup goes through a tightly scoped `SECURITY DEFINER` projection rather than exempting the table from RLS, on the condition that the F-015 gate is extended to assert the function's definition. Per-workspace SSO and MFA are enforced at workspace resolution via step-up, not at login. |
| **ADR-033** | One validated workspace accent; product semantics stay fixed | Accepted | `workspace.settings.appearance.primary_color` is the only workspace colour input, defaults to `#2563EB`, and must clear 4.5:1 against white. The API exposes the typed appearance object, never raw settings. The TaskForge mark, focus, and semantic colours remain fixed. The cost is that light brand colours are rejected as action colours. See [54](54-PREMIUM-WEBAPP-DESIGN-SYSTEM.md). |
| **ADR-034** | API image serves SPA files with a constrained history fallback | Accepted | The production image builds the webapp and the Axum process serves its files through `tower-http`. Only HTML `GET`/`HEAD` application navigations fall back to `index.html`; API, health, metrics, and missing assets retain real status codes. This adds one HTTP dependency but keeps the single-node profile to one public process and makes deep links refresh-safe. See [56](56-SPA-SESSION-AND-ROUTE-RESTORATION.md). |

## How the old §18 questions were resolved

The archived drafts ended with nine unanswered questions. Their disposition:

| Old question | Resolved by |
| --- | --- |
| Approve vocabulary; state vs status distinction | [17](17-GLOSSARY.md), [23](23-WORKFLOW-AND-STATE-MACHINE.md) |
| Approve role scopes and constrained-permission set | ADR-004, ADR-005 |
| Multiple assignees in v1? | **ADR-010 — yes**, with optional primary |
| Environments single or multi-select? | **ADR-010 — single** |
| Task key allocation and retention policy | ADR-007, ADR-008, [03](03-DOMAIN-MODEL.md) §Lifecycle |
| Frontend bundle budgets and browser matrix | ADR-024, [18](18-SUPPORT-MATRIX.md) |
| Plugin trust model; defer managed plugins? | ADR-015, ADR-016, ADR-017 |
| Audit retention and IP/device privacy | **ADR-025** |
| Create ADRs for the core decisions | This register |

## Pending / to be written

- **Auth *mechanism*** — not the protocol. This bullet used to read "session
  cookie vs bearer, refresh strategy, OIDC claim mapping", and
  [40](40-IDENTITY-AUTH-AND-SESSION.md) decides all three explicitly: cookie for
  browsers and bearer for machine actors (:14-18, :26), no refresh strategy at
  all because the pattern is rejected by name (:29-36), and a
  `{ email, name, groups }` claim mapping with authoritative group→role sync
  (:76-88). Describing them as open invited a redesign of a finished document.

  What is actually unsettled is the mechanism layer where that prose meets the
  **already-`Gated` schema**, and in four places the two contradict each other.
  Now drafted as **ADR-032**, status `Proposed`, with the reasoning in
  [40](40-IDENTITY-AUTH-AND-SESSION.md) §Mechanism. To be Accepted at Phase 0.
- **SSE vs WebSocket for bidirectional features** — SSE is the Phase 1 decision;
  a WebSocket ADR is required if any feature genuinely needs client→server
  streaming (none does yet).
- **Custom field value storage** — the typed table shape, to be Accepted before
  Phase 3 plugin custom fields.
- **External search engine** — only if ADR-014's tripwire fires.
- **Multi-region / data residency** — not designed; will need an ADR before any
  commitment is made to a customer.
