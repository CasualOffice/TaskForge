# 00 — TaskForge Documentation Index

`docs/` is the **source of truth** for TaskForge's design. Code follows docs, not
the other way around. This index is the map.

**TaskForge** is the work-tracking service of the Casual Office suite, alongside
Casual Sheets (OpenCalc) and Casual Editor (OpenDoc). Rust, Apache-2.0, crates
prefixed `casual-task-`.

## How to read this

- **New here?** [01-ORD](01-ORD.md) (what & why) → [02-ARCHITECTURE](02-ARCHITECTURE.md)
  (the shape) → [06-ROADMAP](06-ROADMAP-AND-DELIVERY.md) (the order).
- **Want the decisions?** [08-ADR-REGISTER](08-ADR-REGISTER.md) is the whole set
  in one table.
- **Building something?** Follow [11-DESIGN-FIRST-PROCESS](11-DESIGN-FIRST-PROCESS.md),
  update [14-EXECUTION-TRACKER](14-EXECUTION-TRACKER.md).
- **Working the hard parts?** [04-RBAC](04-RBAC-AND-AUTHORIZATION.md) (permissions),
  [26-SEARCH-INDEXING](26-SEARCH-INDEXING-AND-QUERY.md) (queries and indexes),
  [34-PLUGINS](34-PLUGIN-AND-EXTENSION-ARCHITECTURE.md) (extension),
  [32-TENANCY](32-TENANCY-AND-ISOLATION.md) (isolation).

## The four load-bearing documents

Everything else can change incrementally. These four are expensive to reverse and
should be read before writing any code:

| # | Why it is load-bearing |
| --- | --- |
| [04 — RBAC & Authorization](04-RBAC-AND-AUTHORIZATION.md) | The permission model cannot be changed later without touching every endpoint. Additive union, no deny rules, explainable. |
| [26 — Search, Indexing & Query](26-SEARCH-INDEXING-AND-QUERY.md) | The closed filterable field set and its index inventory. This is what keeps the product fast at 2M tasks. |
| [34 — Plugin & Extension Architecture](34-PLUGIN-AND-EXTENSION-ARCHITECTURE.md) | The open/closed contract. Get the seams wrong and every integration becomes a core patch. |
| [25 — Events, Outbox & Audit](25-EVENTS-OUTBOX-AND-AUDIT.md) | Written from the first mutation. Retrofitting an outbox means auditing every write path. |

## Numbering discipline

- Numbers are **stable and never reused.** A retired doc keeps its number with a
  tombstone; new docs take the next free number.
- Ranges mirror the OpenDoc/OpenCalc layout so the three services feel like
  siblings:
  - **00–19** — foundation, process, top-level architecture.
  - **20–29** — stable contracts (errors, limits, schema, workflow, concurrency,
    events, search, filters, attachments, notifications).
  - **30–49** — architecture pillars (performance, tenancy, plugins, automation,
    identity, frontend, observability, deployment).
  - **50+** — operations and per-feature design notes, added as phases open.

## Index

### Foundation & process (00–19)

| # | Title | Purpose |
| --- | --- | --- |
| 00 | This index | Map of the design record |
| 01 | [Outcome & Requirements](01-ORD.md) | What TaskForge is for, and for whom |
| 02 | [Architecture](02-ARCHITECTURE.md) | Target architecture and principles |
| 03 | [Domain Model](03-DOMAIN-MODEL.md) | The eleven nouns, their invariants and lifecycles |
| 04 | [RBAC & Authorization](04-RBAC-AND-AUTHORIZATION.md) | **The permission resolution algorithm** |
| 05 | [API Specification](05-API-SPEC.md) | REST + SSE contract |
| 06 | [Roadmap & Delivery](06-ROADMAP-AND-DELIVERY.md) | Phases, deliverables, exit gates |
| 07 | [Quality, Security & Compatibility](07-QUALITY-SECURITY-AND-COMPATIBILITY.md) | The non-negotiables, threat model |
| 08 | [ADR Register](08-ADR-REGISTER.md) | Every accepted decision, in one table |
| 09 | [Repository & Contribution](09-REPOSITORY-AND-CONTRIBUTION.md) | Layout, PR contract, clean-room rules |
| 10 | [Project Goal & Standards](10-PROJECT-GOAL-AND-STANDARDS.md) | The bar, and the standards that settle arguments |
| 11 | [Design-First Process](11-DESIGN-FIRST-PROCESS.md) | How work is designed before it is built |
| 12 | [Competitive Analysis](12-COMPETITIVE-ANALYSIS.md) | Jira, Linear, Redmine, Forge, GitHub Apps, OrangeScrum |
| 14 | [Execution Tracker](14-EXECUTION-TRACKER.md) | Live state of all work |
| 15 | [CI & Release Gates](15-CI-AND-RELEASE-GATES.md) | The PR contract |
| 16 | [Documentation Maintenance](16-DOCUMENTATION-MAINTENANCE.md) | Keeping docs and code in sync |
| 17 | [Glossary](17-GLOSSARY.md) | The enforced vocabulary |
| 18 | [Support Matrix](18-SUPPORT-MATRIX.md) | Target vs implemented, per surface |
| 19 | [Workspace Scaffold & Layer Division](19-WORKSPACE-SCAFFOLD-DESIGN.md) | Crates, the dependency DAG, the seams |

### Contracts (20–29)

| # | Title | Purpose |
| --- | --- | --- |
| 20 | [Error-Code Registry](20-ERROR-CODE-REGISTRY.md) | Stable diagnostic codes |
| 21 | [API Limits & Quotas](21-API-LIMITS-AND-QUOTAS.md) | Every bound on every input |
| 22 | [Database Schema](22-DATABASE-SCHEMA.md) | The DDL contract |
| 23 | [Workflow & State Machine](23-WORKFLOW-AND-STATE-MACHINE.md) | Configurable statuses over five permanent states |
| 24 | [Concurrency & Idempotency](24-CONCURRENCY-AND-IDEMPOTENCY.md) | Versions, conflicts, retries, races |
| 25 | [Events, Outbox & Audit](25-EVENTS-OUTBOX-AND-AUDIT.md) | The atomic write and the three streams |
| 26 | [Search, Indexing & Query](26-SEARCH-INDEXING-AND-QUERY.md) | **The complete index inventory** |
| 27 | [Filter Grammar & Saved Views](27-FILTER-AND-SAVED-VIEW-DSL.md) | One grammar for lists, views, and automations |
| 28 | [Attachment Pipeline](28-ATTACHMENT-PIPELINE.md) | Streaming upload, scan, commit |
| 29 | [Notifications & Delivery](29-NOTIFICATIONS-AND-DELIVERY.md) | Reasons, channels, batching |

### Architecture pillars (30–49)

| # | Title | Purpose |
| --- | --- | --- |
| 30 | [Performance & Capacity Targets](30-PERFORMANCE-AND-CAPACITY-TARGETS.md) | The numbers, the corpus, the machine |
| 32 | [Tenancy & Isolation](32-TENANCY-AND-ISOLATION.md) | Two independent mechanisms, neither optional |
| 34 | [Plugin & Extension Architecture](34-PLUGIN-AND-EXTENSION-ARCHITECTURE.md) | **Open for extension, closed for modification** |
| 36 | [Automation Rules](36-AUTOMATION-RULES-DESIGN.md) | When / if / then, with `run_as` and loop guards |
| 38 | [Reporting, Export & Dashboards](38-REPORTING-EXPORT-AND-DASHBOARDS.md) | Export, metrics, and the dashboard model |
| 40 | [Identity, Auth & Session](40-IDENTITY-AUTH-AND-SESSION.md) | Sessions, SSO, MFA, tokens |
| 42 | [Frontend Architecture](42-FRONTEND-ARCHITECTURE.md) | The thin client and its budget |
| 44 | [Product Research and Surface Briefs](44-PRODUCT-RESEARCH-AND-SURFACE-BRIEFS.md) | Who uses this, at what moment, and therefore why each screen exists |
| 45 | [Development Lifecycle and Custody](45-DEVELOPMENT-LIFECYCLE-AND-CUSTODY.md) | Two clocks, the chain of custody, and what the process forces into the model |
| 46 | [Observability & Operations](46-OBSERVABILITY-AND-OPERATIONS.md) | Signals, alerts, runbooks, SLOs |
| 48 | [Deployment Profiles](48-DEPLOYMENT-PROFILES.md) | Single node → scaled, one security model |

### Operations (50+)

| # | Title | Purpose |
| --- | --- | --- |
| 50 | [Runbooks](50-RUNBOOKS.md) | Symptom → diagnosis → action, per incident |
| 52 | [Deployment Guide](52-DEPLOYMENT-GUIDE.md) | How to actually run it: image, compose, upgrade, backup |

## Status

**Documentation phase. No code exists yet.** The design record is complete for
Phases 0–4; see [14-EXECUTION-TRACKER](14-EXECUTION-TRACKER.md) for what is
designed, accepted, and pending, and [06-ROADMAP](06-ROADMAP-AND-DELIVERY.md) for
the build order.

Three decisions remain genuinely open and are tracked as such: auth protocol
specifics (Phase 0), custom-field value storage (before Phase 3), and data
residency (before any customer commitment).

## History

This set replaces earlier drafts that described a **Java/Spring** backend and
left nine architectural decisions explicitly unanswered. The Rust decision
(ADR-001) had been made but never propagated to four of the six documents, so the
repository described two incompatible architectures at once.

Those drafts are in [`_archive/`](_archive/README.md) with a note on what went
wrong and why this set has a single owner per fact
([16](16-DOCUMENTATION-MAINTENANCE.md)).
