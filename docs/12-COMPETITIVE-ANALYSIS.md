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
