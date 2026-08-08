# 10 — Project Goal & Standards

## The goal, in one paragraph

Build a work tracker whose **core stays small permanently**, because extension
happens at declared seams instead of inside the core — and which is nonetheless
correct about the things trackers get wrong: who may do what, what actually
changed, and how to find anything at scale. Simple enough that a team is
productive in ten minutes; rigorous enough that the same team, five years and
two million tasks later, has not outgrown it.

## The bar

**Production-grade.** Not an MVP, not a prototype. Specifically:

- A permission decision is never wrong, and is always **explainable**.
- Data never crosses a workspace boundary.
- A change that was accepted is never silently lost.
- Every history record is immutable and attributable.
- No user-reachable query performs a sequential scan.
- No plugin can fail, block, or slow a core request.
- One binary and PostgreSQL is a real, supported, secure deployment.

## The four standards that decide arguments

When a decision is contested, these settle it — in order.

### 1. Adding a capability must not add a concept

The simplicity contract ([01](01-ORD.md)). A feature earns its place by fitting
an existing noun — a task type, a status, a permission, an extension point, a
filter field. If it needs a new top-level noun in the user's vocabulary, it needs
an ADR arguing the noun is unavoidable ([17](17-GLOSSARY.md)).

This is why there are no sprints, no epics, and no issue-vs-task distinction.

### 2. The server decides; the client renders

Authorization, filtering, sorting, pagination, and workflow validation are server
concerns. A hidden button is presentation, never security
([04](04-RBAC-AND-AUTHORIZATION.md)).

### 3. Make the wrong thing impossible, not discouraged

Prefer a mechanism to a rule:

| Instead of the rule | The mechanism |
| --- | --- |
| "always filter by workspace" | `WorkspaceScope` required by every repository method |
| "don't publish events outside the transaction" | handlers return events; they have no publisher |
| "activity is append-only" | `UPDATE`/`DELETE` revoked from the application role |
| "don't query unindexed fields" | closed field set; unknown field is a `400` |
| "don't let uncommitted attachments leak" | partial index excludes them |
| "modules shouldn't reach into each other" | crate boundaries; illegal dependency is a build error |

A rule survives until the eleventh engineer. A compile error survives.

### 4. State the cost

Every trade-off has a losing side. Name it in the doc
([16](16-DOCUMENTATION-MAINTENANCE.md)). The additive-RBAC cost in
[04](04-RBAC-AND-AUTHORIZATION.md) is the model: the limitation is stated plainly,
with the workaround, so nobody rediscovers it as a surprise under a deadline.

## Engineering priorities (ordered)

Restated from [07](07-QUALITY-SECURITY-AND-COMPATIBILITY.md); when two conflict,
the earlier wins:

1. Correctness & authority
2. Tenant isolation
3. Traceability
4. Security & resource bounds
5. Data durability
6. Performance
7. API stability
8. UX
9. Maintainability

Note where UX sits. It is high — this product's differentiation depends on feel —
but it never outranks a permission decision or an audit record.

## Code standards

- Rust 2024; `unsafe_code = "forbid"`; clippy `-D warnings`.
- **All SQL compile-checked** via `sqlx::query!`.
- Errors are typed and carry a registry code ([20](20-ERROR-CODE-REGISTRY.md)).
  `unwrap()` in non-test code requires a comment proving it cannot panic.
- No `#[allow(...)]` without a comment explaining why.
- Public items documented; the SDK surface deliberately narrower than internals.
- Tests with every behaviour change. Property tests where an invariant exists.
- One coherent capability per PR.

## What we will not do

- Ship a feature without its acceptance gate.
- Add a query path without its index.
- Add a user-facing noun without an ADR.
- Run customer code in the API process.
- Claim "lossless", "seamless", or "simply" in a document
  ([16](16-DOCUMENTATION-MAINTENANCE.md)).
- Copy source, schema, templates, or assets from another tracker
  ([09](09-REPOSITORY-AND-CONTRIBUTION.md)).
- Disable a CI gate to ship. If a gate is wrong, change it by ADR, in the open.

## Relationship to the suite

TaskForge is the work-tracking service of **Casual Office**, alongside Casual
Sheets (OpenCalc) and Casual Editor (OpenDoc). Shared: the design system
(`@schnsrw/design-system`), the Rust toolchain and CI shape, the numbered-docs
process, and Apache-2.0.

**Not shared:** the runtime. OpenDoc and OpenCalc are embeddable engines;
TaskForge is a service with a database and a permission model. Borrow their
process, not their architecture — they solve a different problem.
