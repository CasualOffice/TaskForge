# 01 — Outcome & Requirements (ORD)

What TaskForge is for, who it serves, and the requirements that define "done."

## Problem

Team work tracking today forces a bad trade:

- **Heavyweight trackers** (Jira, Azure DevOps) can model any process, but the
  cost is permanent: every team pays for configuration surface it will never use,
  and "simple" workflows still route through screens, schemes, and contexts.
- **Lightweight trackers** (Linear, Trello, Basecamp) feel good until the first
  real requirement lands — a per-project role, a QA gate, an environment,
  an auditable history — and then there is no seam to add it.
- **Self-hosted open source** (OrangeScrum, Redmine, Taiga) gets you control, but
  extension usually means forking the core, and the fork can never be upgraded.

The missing product is a tracker whose **core stays small permanently** because
extension happens at declared seams rather than inside the core. Simple to
understand on day one; still correct when a team needs project-scoped roles,
a custom workflow, and a compliance audit trail.

## Outcome

A team can:

1. **Model work** as one universal item — task, bug, feature, incident, request
   are *types*, not separate incompatible entities.
2. **Track it** through a workflow they configured, whose statuses map onto five
   stable semantic states that APIs, reports, and plugins can rely on forever.
3. **Control who can do what** with roles assigned per project (not just per
   workspace), evaluated on the server for every mutation, and explainable —
   a user can ask *why* they were denied and get a real answer.
4. **Find anything** — full-text across tasks and comments, structured filters
   over every indexed field, saved as reusable views, always permission-filtered
   and always served from an index rather than a scan.
5. **See the whole history** — an append-only, human-readable activity stream and
   a separate security-grade audit stream with independent retention.
6. **Extend it without forking** — plugins add panels, actions, fields, commands,
   automations, and integrations through declared extension points, with scoped
   permissions, admin consent, and hard failure isolation.
7. **Run it themselves** — one Rust binary plus PostgreSQL, with Redis and object
   storage optional until scale demands them.

## Users

- **Delivery teams** (5–500 people) who need real workflow and real permissions
  without a full ALM rollout.
- **Self-hosting organizations** who need the data on their own infrastructure
  under Apache-2.0, with SSO, audit, and export.
- **Integrators** building on top: plugin authors and API consumers who need a
  contract that does not shift under them.
- **The Casual Office suite** — TaskForge is its work-tracking service, alongside
  Casual Sheets (OpenCalc) and Casual Editor (OpenDoc).

## Requirements

### Functional

- **FR-1** Workspaces as tenant boundary; users belong to many workspaces.
- **FR-2** Projects as collaboration boundary, with private / team / workspace
  visibility.
- **FR-3** Tasks with subtasks, dependencies, tags, milestones, environments,
  multiple assignees, and a human-readable key (`WR-125`).
- **FR-4** Configurable statuses and transitions per workflow, each mapped to one
  of five stable states ([23](23-WORKFLOW-AND-STATE-MACHINE.md)).
- **FR-5** Role-based access control with assignments at workspace, team,
  project, and environment scope; deterministic, explainable resolution
  ([04](04-RBAC-AND-AUTHORIZATION.md)).
- **FR-6** Append-only activity history and a separate audit stream
  ([25](25-EVENTS-OUTBOX-AND-AUDIT.md)).
- **FR-7** Full-text and structured search, permission-filtered, index-served
  ([26](26-SEARCH-INDEXING-AND-QUERY.md)).
- **FR-8** Saved views over a typed filter grammar ([27](27-FILTER-AND-SAVED-VIEW-DSL.md)).
- **FR-9** Attachments via pre-signed streaming upload with a scan/commit
  handshake ([28](28-ATTACHMENT-PIPELINE.md)).
- **FR-10** Notifications with per-user delivery preferences ([29](29-NOTIFICATIONS-AND-DELIVERY.md)).
- **FR-11** Plugin installation, consent, scopes, and lifecycle
  ([34](34-PLUGIN-AND-EXTENSION-ARCHITECTURE.md)).
- **FR-12** Rule-based automation over domain events ([36](36-AUTOMATION-RULES-DESIGN.md)).
- **FR-13** Live updates via SSE; REST for all commands and reads ([05](05-API-SPEC.md)).

### Non-functional

- **NFR-1 Authority** — every mutation authorized server-side against the actor's
  effective permissions. A hidden button is never a control.
- **NFR-2 Tenant isolation** — no query, cache key, object key, index document,
  or background job can address data without a workspace scope
  ([32](32-TENANCY-AND-ISOLATION.md)).
- **NFR-3 Traceability** — every material change produces an immutable history
  record in the same transaction as the change itself.
- **NFR-4 Latency** — p95 read < 150 ms, p95 write < 300 ms server-side at
  reference capacity ([30](30-PERFORMANCE-AND-CAPACITY-TARGETS.md)).
- **NFR-5 Index discipline** — no user-reachable query performs a sequential scan
  on a tenant-scale table; sort and filter fields are a **closed, indexed set**
  ([26](26-SEARCH-INDEXING-AND-QUERY.md)).
- **NFR-6 Client weight** — authenticated shell ≤ 200 KB compressed JS, enforced
  in CI ([42](42-FRONTEND-ARCHITECTURE.md)).
- **NFR-7 Extension isolation** — no plugin can block, slow past its timeout, or
  fail a core request; no plugin touches the core database
  ([34](34-PLUGIN-AND-EXTENSION-ARCHITECTURE.md)).
- **NFR-8 Operability** — one binary + PostgreSQL is a valid deployment;
  everything else is opt-in ([48](48-DEPLOYMENT-PROFILES.md)).
- **NFR-9 Legal cleanliness** — Apache-2.0. Clean-room: no OrangeScrum (or any
  other tracker's) source, assets, templates, or schema is copied.

## Non-goals

Deliberately out of scope, so the core stays small:

- CRM, payroll, invoicing, portfolio finance.
- Chat, video meetings, whiteboards, document editing (those are other Casual
  Office services; TaskForge links to them, it does not become them).
- Arbitrary customer code executing **inside** the core process — ever. Extension
  runs out-of-process or in a sandbox ([34](34-PLUGIN-AND-EXTENSION-ARCHITECTURE.md)).
- Microservice-per-module. TaskForge is a modular monolith ([02](02-ARCHITECTURE.md)).
- Offline-first sync. A draft cache and retry queue are in scope; a replication
  engine is not.
- Gantt/portfolio/resource-management as core features — plugin surface.

## Definition of done (service level)

A self-hosted team can run one binary against PostgreSQL, create a workspace and
projects, configure roles and a workflow, track work through it, search and
filter across everything from an index, read a complete audit trail, install a
plugin that adds a panel and an automation — and never once needs to modify or
rebuild the core to do it.

## The simplicity contract

The single hardest requirement in this document, and the one most likely to be
violated quietly:

> **Adding a capability must not add a concept.**

A new feature earns its place by fitting an existing concept (a task type, a
status, a permission, an extension point, a filter field). If it needs a new
top-level noun in the user's vocabulary, it needs an ADR arguing why the noun is
unavoidable. [17-GLOSSARY](17-GLOSSARY.md) is the enforced vocabulary; if a term
is not there, users should not have to learn it.
