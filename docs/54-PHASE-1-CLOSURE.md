# 54 — Phase 1 Closure

Phase 1 has enough breadth to expose the product, but its first twenty-one core
rows do not all have the acceptance evidence required by [06](06-ROADMAP-AND-DELIVERY.md)
and [15](15-CI-AND-RELEASE-GATES.md). This note defines the closure order. It
adds no user-facing capability and settles no open product decision.

## Failure this prevents

Later capabilities have landed while identity, authorization, tenancy,
workflow, task history, filtering, pagination and notification rows still read
`Building`. Continuing in that order makes the tracker a catalogue of shipped
surfaces rather than a dependency-ordered delivery plan. It also lets a broad
green test job conceal that a named acceptance property has no exhaustive gate.

Phase 1 therefore closes depth-first. No new Phase 2–4 capability enters the
core until the rows below are either `Gated` or truthfully blocked by an accepted
decision or an external measurement named in the tracker.

## Closure order

The order follows the engineering priorities in [10](10-PROJECT-GOAL-AND-STANDARDS.md):
authority and isolation precede traceability, performance, contracts and UX.

| Order | Rows | Closure evidence |
| --- | --- | --- |
| 1 | C-001–C-005 | Authentication seam definition, permission matrix, all seven escalation controls, additivity/isolation properties, and route-derived 404/cross-tenant coverage |
| 2 | C-007–C-011 | Workflow and task command invariants; comments and attachments; one-transaction activity, audit and outbox records; worker consumers exercised against PostgreSQL |
| 3 | C-012–C-016 | Closed filter and sort sets, one authorization resolution per list, cursor properties, search plan evidence, SSE revocation and notification delivery |
| 4 | C-017–C-021 | Browser-readable extension registry, core-flow browser coverage, accessibility automation, bounded rate limits and export lifecycle |

C-006 and C-015 are already `Gated`; their gates remain in the sequence because
the route-wide suites depend on them.

## A row is not closed by existence

A handler, repository method or test file proves that work exists. It does not
prove the row is gated. Moving a row to `Gated` requires all of the following:

1. The behavior is reachable through the public surface named in its design
   note.
2. The row's security and concurrency invariants have negative tests that fail
   when the protection is removed.
3. The tests run in a blocking CI job; ignored Docker tests are invoked
   explicitly.
4. Every open decision named as a precondition has an Accepted ruling. A safe
   current behavior may remain `Built`, but it is not silently promoted.
5. The tracker states any remaining non-gated release condition, including a
   human check or reference-machine measurement.

## Route-derived authority coverage

The router is the public tenant surface. A hand-maintained list of endpoints in
a test will omit the next route added. The authority gate therefore derives the
registered `/api/v1` route templates and requires each tenant-scoped route to
declare one of these dispositions:

- cross-tenant refusal tested;
- invisible and absent resources proven indistinguishable;
- pre-workspace endpoint covered by the fixed credential projection;
- actor-only endpoint whose response cannot name another subject;
- public health or authentication endpoint with no tenant row access.

An unclassified route fails CI. The disposition registry is test metadata, not
runtime authority; handlers still authorize every request and PostgreSQL RLS
remains the independent backstop.

## Authorization cache

[04](04-RBAC-AND-AUTHORIZATION.md) requires an in-process cache keyed by
`(workspace_id, actor_id, project_id, authz_epoch)`. It may serve read
affordances and accessible-project discovery only. Mutations continue resolving
inside their transaction. The cache is bounded, has a 60-second TTL, and cannot
construct a key without a workspace id and epoch.

Acceptance requires:

- a repeated read at one epoch hits the cache;
- an epoch bump makes the old entry unreachable without waiting for TTL;
- identical actor and project ids in different workspaces never share an entry;
- mutation authority does not call the cache;
- one full 100-task page performs one authority resolution.

## Decisions that implementation must not guess

These existing tracker decisions remain explicit preconditions:

| Decision | Blocks |
| --- | --- |
| D-056 — exact built-in template permission sets | C-004 `Gated` |
| D-057 — permission governing membership, teams and invitations | C-002 `Gated` |
| D-059 — notification preferences, subscriptions, quiet hours and digests | C-016 `Gated` |
| D-064 — MFA step-up lifetime | C-001 `Gated` |

D-051 and D-043 block complete query/search claims and are handled with the
Phase 1 exit evidence rather than weakened here.

## Tracker reconciliation

The tracker is reconciled from executable evidence, not commit subjects. A row
may move from `Building` to `Built` when its complete behavior and tests are
merged. It moves from `Built` to `Gated` only when the blocking workflow invokes
its acceptance suite. Stale prose naming a route or worker as absent is removed
in the same increment that records the evidence proving it present.

## Cost

This closure pass delays new visible features and spends review capacity on
tests, status reconciliation and module boundaries. The losing side is roadmap
velocity measured by screens. The gain is that Phase 1 closes on the product's
authority, isolation and durability promises instead of on feature count.
