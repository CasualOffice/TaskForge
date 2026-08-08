# 12 — Competitive Analysis

What we study, why, and what we take. Following the house rule
([16](16-DOCUMENTATION-MAINTENANCE.md)): every claim carries a **date checked**,
and this survey must be **re-verified before it is relied on for a specific
decision**. Product capabilities and pricing move fast; the *architectural
patterns* below are the durable part.

**Initial survey checked: 2026-08-08.** Claims about specific vendor features are
from public documentation and general industry knowledge; treat feature-level
specifics as *needing confirmation*, and the structural observations as the
usable content.

We study three kinds of prior art: **the complexity ceiling** (what happens when
you say yes to everything), **the simplicity ceiling** (what happens when you say
no to everything), and **extension models** (how others solved the problem this
product is built around).

## The complexity ceiling

### Jira — the configurability oracle

- **Role:** the definition of "can model any process," and the definition of the
  cost of that. Screens, field configurations, issue type schemes, permission
  schemes, notification schemes, workflow schemes — each individually reasonable,
  collectively a full-time administrator.
- **Take:** the **status/state distinction**. Jira's status categories (To Do /
  In Progress / Done) exist because reporting and automation cannot depend on
  customer-renamed statuses. TaskForge takes the idea and makes it stricter —
  five states, fixed forever, in the API from day one
  ([23](23-WORKFLOW-AND-STATE-MACHINE.md)). We also take the transition-as-command
  model: you cannot write a status field directly.
- **Deliberately reject:** scheme indirection. Jira separates permission scheme
  from role from group from project role, so answering "why can this person do
  this" requires traversing four indirections. TaskForge's additive-union model
  answers it with a flat list of grants ([04](04-RBAC-AND-AUTHORIZATION.md)).
- **The lesson we encode:** configurability is not the problem — *unbounded
  configurability with no simple default* is. Our default workflow works with
  zero configuration, and every advanced surface is progressively disclosed.

### Azure DevOps / ServiceNow — the enterprise ceiling

- **Role:** evidence of what happens when process modelling becomes the product.
- **Take:** audit and compliance expectations (immutable history, separable
  retention, exportability) are real and cannot be retrofitted — which is why
  [25](25-EVENTS-OUTBOX-AND-AUDIT.md) is Phase 1, not Phase 4.
- **Reject:** the assumption that every organization wants to model its process
  before doing work.

## The simplicity ceiling

### Linear — the UX and performance oracle

- **Role:** the current bar for how a tracker should *feel* — command palette,
  keyboard-first, optimistic updates, sub-100 ms interactions, an opinionated
  default workflow.
- **Take:** the interaction model wholesale — command palette as the primary
  action surface ([42](42-FRONTEND-ARCHITECTURE.md)), optimistic mutation with
  version-aware rollback, side-drawer detail that preserves board context, and the
  discipline that the client stays thin and fast.
- **Where we differ:** Linear's opinionation is its product. TaskForge needs
  per-project roles, configurable workflows, environments, and self-hosting —
  which Linear declines by design. Our bet is that you can have Linear's *feel*
  with configurable authority underneath, if the configuration is
  progressively disclosed rather than front-loaded.
- **Honest risk:** this is the hardest claim in the product. Every tracker that
  added configurability lost the feel. The mitigations are the simplicity
  contract in [01](01-ORD.md) and the bundle/latency gates in
  [30](30-PERFORMANCE-AND-CAPACITY-TARGETS.md) — enforced in CI, so the loss shows
  up as a failing build rather than a slow drift.

### Trello / Basecamp / Height — the minimal end

- **Role:** proof that most teams need far less than they think, and that the
  first-run experience determines adoption.
- **Take:** the create-task form asks for a title and nothing else. Everything
  further is disclosed on demand.
- **Reject:** the board-as-the-only-model. TaskForge's board is a *view* over a
  workflow, not the data model — which is why List, My Work, and reporting are
  not bolted on.

### Notion / Airtable — the flexible-database end

- **Role:** the strongest argument for user-defined fields and views, and the
  clearest demonstration of the cost: when everything is a database, nothing has
  domain semantics, and the tool cannot help you.
- **Take:** typed custom fields as an *extension point*
  ([34](34-PLUGIN-AND-EXTENSION-ARCHITECTURE.md)), not as the core model.
- **Reject:** schemaless core. Our core is normalized and typed; JSONB is confined
  to plugin metadata and validated custom-field values
  ([22](22-DATABASE-SCHEMA.md)).

## Extension models — the most important comparison

This is where TaskForge's central bet lives, so it gets the closest study.

### Redmine — plugins as Rails monkey-patching

- **Pattern:** plugins load into the application process and patch core classes.
- **Outcome:** extremely capable *and* the reason Redmine upgrades are famously
  painful. A plugin couples to internals; a core refactor breaks the ecosystem;
  the core therefore stops refactoring.
- **What we take:** the negative lesson, directly. This is the exact failure
  ADR-009 is written to prevent — a **closed, typed extension point registry**
  instead of open access to internals.

### OrangeScrum — the clean-room reference point

- **Role:** the product whose *category* TaskForge occupies, and the reason the
  clean-room constraint exists.
- **Position:** we study its **feature surface** to understand market
  expectations. We copy **no source, schema, template, or asset**
  ([01](01-ORD.md) NFR-9, [09](09-REPOSITORY-AND-CONTRIBUTION.md)).
- **Take:** confirmation of the baseline feature set self-hosting teams expect
  (projects, tasks, time, milestones, roles) — as a checklist of *what to have an
  answer for*, not as a design to imitate.

### Jira Forge / Connect — the mature app platform

- **Pattern:** Connect = remote HTTPS apps with signed requests; Forge = hosted
  functions plus a sandboxed UI (UI Kit / iframe), with declared scopes and admin
  consent.
- **Take, substantially.** This is the closest prior art to
  [34](34-PLUGIN-AND-EXTENSION-ARCHITECTURE.md), and its structure is sound:
  manifest-declared modules, scope consent, independently versioned platform
  contract, sandboxed frontends.
- **Where we simplify:** Atlassian ships two overlapping platforms with a long
  migration between them. TaskForge starts with the declarative and remote
  classes and *defers* managed workers (ADR-016) rather than shipping two models
  and reconciling later.

### GitHub Apps — the scope/consent oracle

- **Take:** fine-grained, least-privilege scopes; per-installation tokens;
  re-consent on scope escalation; installation-level (not user-level) identity.
  All four are in our model.
- **Also take:** the webhook delivery discipline — signed payloads, delivery logs
  the customer can inspect, redelivery. Our per-installation observability panel
  is modelled on it.

### Slack — the consent-UX oracle

- **Take:** the install screen that states, in plain language, exactly what the
  app can see and do. Consent is a *product surface*, not a legal checkbox. Ours
  shows scopes, extension points, declared retention, declared PII, and egress
  allow-list before anything is granted.

## Reporting, dashboards & export

The area with the widest quality spread in the market, and the one where the
cheap version is worst. Surveyed for [38](38-REPORTING-EXPORT-AND-DASHBOARDS.md).

### Jira dashboards & JQL — the power/comprehensibility trade

- **Pattern:** gadget-based dashboards over JQL, plus a separate paid product
  (Atlassian Analytics / Data Lake) once anyone wants real analysis.
- **Take:** the confirmation that **one query language across search, saved
  views, automation, and reports** is the right shape — JQL's reach is exactly
  why it survived. TaskForge takes the idea and rejects the syntax: a typed AST
  over a closed field set ([27](27-FILTER-AND-SAVED-VIEW-DSL.md)) gives the same
  reuse without a language users must learn or an unbounded query planner.
- **Reject:** the gadget zoo, and the fact that a serious question requires
  leaving the product. Our answer to "I need real BI" is a first-class **export**
  ([38](38-REPORTING-EXPORT-AND-DASHBOARDS.md)), shipped two phases before
  reports, rather than a half-built warehouse.

### Linear — the "few, correct metrics" position

- **Pattern:** a small set of opinionated insights (cycle time, throughput,
  scope change) rather than a report builder.
- **Take:** substantially. Six good visualizations beat a chart builder nobody
  can drive, and the built-in dashboards in [38](38-REPORTING-EXPORT-AND-DASHBOARDS.md)
  are expressed in the same model users get — which is the proof the model is
  sufficient.
- **Take also:** cycle time computed from **state history**, not from a
  `resolved_at` column. This is why `task_state_interval` exists rather than a
  timestamp pair.

### Notion / Airtable — the flexible-view trap

- **Pattern:** any view, any grouping, any rollup, user-defined.
- **Take:** grouping and bucketing as first-class report parameters.
- **Reject:** unbounded user-defined aggregation. It is the single clearest path
  to a query nobody indexed, and it breaks the promise in
  [26](26-SEARCH-INDEXING-AND-QUERY.md) that no user-reachable query scans.

### Metabase / Superset — what a real BI tool looks like

- **Role:** the honest comparison for "can't you just add dashboards?" They are
  large products with query builders, caching layers, and permission models of
  their own.
- **Take:** the boundary. We are not competing here, and pretending otherwise
  would produce a bad tracker *and* a bad BI tool. Instead: export cleanly, and
  stream events to the customer's warehouse via webhooks.

### Excel — the actual competitor for reporting

- **Observation, not a joke:** the overwhelming majority of tracker "reports" end
  as an export someone pivots by hand. It is the reason **export ships in Phase 2
  and reports in Phase 4**, not the other way around.
- **Take:** CSV that Excel opens correctly (UTF-8 BOM, RFC 4180), and `.xlsx`
  through OpenCalc — the suite's own engine, so the format costs a dependency
  edge rather than a vendor.
- **Take the warning too:** CSV formula injection (`=cmd|'/c calc'!A1` in a task
  title) is a live, widely-shipped vulnerability in this exact feature. It has
  its own non-negotiable test in [38](38-REPORTING-EXPORT-AND-DASHBOARDS.md).

## Where TaskForge deliberately differs

| Axis | Most prior art | TaskForge |
| --- | --- | --- |
| Permission model | allow + deny with precedence, or flat roles | additive union, no deny, **explainable** ([04](04-RBAC-AND-AUTHORIZATION.md)) |
| "Why was I denied?" | read the schemes | `POST /permissions/explain` returns the grants |
| Status vs state | conflated, or category as an afterthought | five fixed states are the permanent API contract |
| Extension | in-process hooks (Redmine) or a second platform (Atlassian) | one closed, typed registry; core features use it too |
| Search scope | grows organically, indexed reactively | **closed filterable field set, index enumerated per field** ([26](26-SEARCH-INDEXING-AND-QUERY.md)) |
| Client weight | grows with feature count | ≤ 200 KB gated in CI ([42](42-FRONTEND-ARCHITECTURE.md)) |
| Self-hosting | full compose stack required | one binary + PostgreSQL ([48](48-DEPLOYMENT-PROFILES.md)) |
| Ordering | float ranks (precision-fail) or integers (renumber) | lexicographic ranks (ADR-013) |
| Reporting | a report builder, or a separate paid BI product | closed measure set over the same filter grammar; **export first** ([38](38-REPORTING-EXPORT-AND-DASHBOARDS.md)) |
| Export | an afterthought, often synchronous and unaudited | async, streamed, permission-checked per batch, audited, injection-safe |

## What the survey says we are most likely to get wrong

Recording the risks now, so they are testable later rather than rediscovered:

1. **Feel vs configurability.** Every product in the "complexity ceiling" section
   started simple. Our only real defense is that the gates are automated
   ([15](15-CI-AND-RELEASE-GATES.md)) — opinion does not scale, CI does.
2. **The additive-permission trade.** No deny rules is the right default, but the
   first enterprise deal that demands "everyone except contractors" will pressure
   it. The answer is already designed (private projects + teams); it needs to be
   *documented for sales*, not re-litigated in code.
3. **The 200 KB budget.** React + TanStack Query + Router + dnd-kit + virtualization
   is most of the budget before product code. This must be measured in Phase 0
   with a real dependency set, and the number adjusted by ADR if the floor is
   genuinely higher — not quietly exceeded ([42](42-FRONTEND-ARCHITECTURE.md)).
4. **Extension points chosen too narrowly.** If the first ten real integrations
   each need a new point, the registry was wrong. Phase 3 should be preceded by
   designing three real plugins against the contract on paper.

## Open questions to resolve with research

- Re-verify current Jira Forge module and scope taxonomy before finalizing our
  scope list — it is the closest thing to a de-facto standard.
- Survey self-hosted deployment expectations (SSO protocols in practice, backup
  tooling, upgrade cadence tolerance) with actual self-hosters, not assumptions.
- Confirm which "advanced" features teams abandon trackers *for* (time tracking?
  capacity planning? reporting depth?) to decide what must be a first-class
  plugin at launch rather than a later one.
- Benchmark a real React shell with our exact dependency set against the 200 KB
  budget, before Phase 1 commits to it.
