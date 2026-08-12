# AGENTS.md — Agent Contract for TaskForge

This file is the entry contract for **every** coding agent that works in this
repository. Read it in full before doing anything else.

## Module size and shape

**No file over ~500 lines.** Not a style preference — a measured failure mode in
this repository. `tasks.rs` reached 1,757 lines with nine handlers and twelve
helpers, and two of those handlers had already grown slightly different ways of
deciding visibility. Nobody reads a file that size; they grep it, land in the
middle, and change what they find.

**Split by reason to change, not by size.** Cutting a long file in half at the
midpoint produces two files that still change together. The question is what
makes each part change:

| Module | Changes when |
| --- | --- |
| `wire` | the API contract does (`docs/05`) |
| `validate` | a field rule does |
| `guard` | authority or visibility does (`docs/04`) |
| handlers | the behaviour does |

The `guard` split carries most of the weight: it exists so "may this actor do
this" cannot be assembled two different ways in two handlers, which is how one
endpoint ends up more permissive than the one beside it.

**Single responsibility, applied to the failure.** A module earns its existence
by naming a failure it prevents, and its doc comment says which. A module whose
doc comment can only describe what it contains — "helpers", "utils", "common" —
has not been given a responsibility, it has been given a leftovers drawer.

**Dependency inversion where a boundary is real.** `Mailer` is a trait in
`casual-task-infra` with an SMTP implementation and a logging one, because
`docs/48` makes email optional and the single-node profile must work without a
relay. `Consumer` in the worker is a trait so the dispatch loop can be tested
without a network. Both exist because the seam is genuine — not because
indirection is virtuous.

**Interface segregation over convenience.** `Authenticated` and
`WorkspaceMember` are separate extractors precisely so a handler that only needs
to know *who* cannot reach tenant data: it has no `AuthContext` to build a scope
from. One combined type would have made that a matter of discipline.


## Repository boundary

- **This repository (`tasks/`, product name TaskForge) is the target.** All work
  happens here.
- **OpenDoc and OpenCalc are reference-only.** They are the sibling Casual Office
  engines — separate repositories, checked out alongside this one as `../opendoc`
  and `../sheets` in the development monorepo. Read them to learn the
  numbered-docs process, the CI gate shape, and the workspace conventions.
  **Do not modify them.**
- **The design system is a consumed dependency** (`@schnsrw/design-system`,
  `../design-system` locally), not a work surface.

If you have cloned TaskForge on its own, those paths will not exist. Nothing in
this repository depends on them — they are context, not build inputs.

**Borrow their process, not their architecture.** OpenDoc and OpenCalc are
embeddable, deterministic, single-process engines. TaskForge is a multi-tenant
service with a database, a permission model, and untrusted extensions. Copying
their layering reasoning into this repository produces the wrong answer.

## Mission

Build a **production-grade** work-tracking service whose core stays small
permanently, because extension happens at declared seams instead of inside the
core — and which is nonetheless correct about the things trackers get wrong: who
may do what, what actually changed, and how to find anything at scale.

Not an MVP. Not a prototype. The bar is in
[docs/10-PROJECT-GOAL-AND-STANDARDS.md](docs/10-PROJECT-GOAL-AND-STANDARDS.md).

## Prime directive: design it right the first time

TaskForge is designed **fully and correctly up front** so later phases slot in
without rework. The order of *construction* is phased
([docs/06-ROADMAP-AND-DELIVERY.md](docs/06-ROADMAP-AND-DELIVERY.md)); the *design*
is not deferred.

Four things are fixed in Phase 0–1 even though their consumers arrive much later:

- **The permission resolver** — Phase 1 ships built-in roles only, but through the
  **final** algorithm, so Phase 2 custom roles add *data*, not an engine
  ([docs/04-RBAC-AND-AUTHORIZATION.md](docs/04-RBAC-AND-AUTHORIZATION.md)).
- **The transactional outbox** — written from the first mutation, when SSE is the
  only consumer ([docs/25-EVENTS-OUTBOX-AND-AUDIT.md](docs/25-EVENTS-OUTBOX-AND-AUDIT.md)).
- **The extension point registry** — core panels render through it before any
  plugin exists ([docs/34-PLUGIN-AND-EXTENSION-ARCHITECTURE.md](docs/34-PLUGIN-AND-EXTENSION-ARCHITECTURE.md)).
- **The index and filter contract** — defined before the first list endpoint
  ([docs/26-SEARCH-INDEXING-AND-QUERY.md](docs/26-SEARCH-INDEXING-AND-QUERY.md)).

If a design decision would force a later rewrite of a lower layer, it is wrong —
stop and redesign before writing code.

**This repository has already paid for ignoring that once.** The drafts in
`docs/_archive/` described a Java/Spring backend that had been superseded by a
Rust decision the author never propagated. Six documents, two incompatible
architectures, and the wrong one was the one you would have opened first.

## Required workflow

For any non-trivial change, in order:

1. **Read the docs.** Start at [docs/00-README.md](docs/00-README.md); read the
   design notes and ADRs that touch your area.
2. **Design first.** Write or update a numbered design note in `docs/` before
   implementing. If the change trips an ADR trigger
   ([docs/11-DESIGN-FIRST-PROCESS.md](docs/11-DESIGN-FIRST-PROCESS.md)), write the
   ADR and get it **Accepted** first.
3. **Discuss and finalize substantial designs** before building them.
4. **Update the execution tracker**
   ([docs/14-EXECUTION-TRACKER.md](docs/14-EXECUTION-TRACKER.md)) — add or move the
   row, using the controlled status vocabulary.
5. **Implement in small, reviewable increments.** One coherent capability per PR.
6. **Add tests with every behaviour change.** Authorization, tenancy, and
   concurrency tests are not optional.
7. **Update docs and ADRs** so the written design and the code never diverge.
8. **Keep CI current.** If a gate should exist and doesn't yet, document the gap
   and add it to the future-gates list in
   [docs/15-CI-AND-RELEASE-GATES.md](docs/15-CI-AND-RELEASE-GATES.md).

## Engineering priorities (ordered)

When two goals conflict, the earlier one wins:

1. **Correctness & authority** — never grant access that was not granted; never
   lose a change that was accepted.
2. **Tenant isolation** — no data crosses a workspace boundary, ever.
3. **Traceability** — every material change is attributable and immutable.
4. **Security & resource bounds** — every input bounded, every external call
   timed, no customer code in-process.
5. **Data durability** — backups verified by restore, not by existence.
6. **Performance & scale** — the gated targets in
   [docs/30-PERFORMANCE-AND-CAPACITY-TARGETS.md](docs/30-PERFORMANCE-AND-CAPACITY-TARGETS.md).
7. **API stability** — the public surface is narrower than internals, and versioned.
8. **UX** — fast, keyboard-first, progressively disclosed.
9. **Maintainability.**

Note where UX sits: high, because this product's differentiation depends on feel —
but never above a permission decision or an audit record.

## Design rules

- **The server decides; the client renders.** Authorization, filtering, sorting,
  pagination, and workflow validation are server concerns. A hidden button is
  presentation, never security.
- **Make the wrong thing impossible, not discouraged.** Prefer a mechanism to a
  rule: `WorkspaceScope` required by every repository method; handlers return
  events instead of holding a publisher; `UPDATE`/`DELETE` revoked on audit
  tables; unknown filter fields are a `400`. A rule survives until the eleventh
  engineer; a compile error survives.
- **One command, one transaction, one history record.** The domain change, its
  activity record, its audit record, and its outbox event commit together or not
  at all.
- **Status is never written directly.** Transitions are commands
  ([docs/23-WORKFLOW-AND-STATE-MACHINE.md](docs/23-WORKFLOW-AND-STATE-MACHINE.md)).
- **No query path without its index.** Adding a filterable or sortable field means
  adding its index and its `EXPLAIN` assertion **in the same PR**.
- **No I/O inside a transaction.** No HTTP, no object store, no plugin call.
- **No customer code in the API process.** Ever.
- **Adding a capability must not add a concept.** A new user-facing noun is an ADR
  trigger ([docs/17-GLOSSARY.md](docs/17-GLOSSARY.md)).
- **State the cost.** Every trade-off has a losing side; name it in the doc.

## Verification rules

- Run the relevant gates before claiming a task done. See
  [CONTRIBUTING.md](CONTRIBUTING.md) for exact commands and
  [docs/15-CI-AND-RELEASE-GATES.md](docs/15-CI-AND-RELEASE-GATES.md) for the full
  contract.
- **"Done" means `Gated`, not `Built`** — merged, tested, *and* protected by an
  acceptance gate that will catch a regression
  ([docs/14-EXECUTION-TRACKER.md](docs/14-EXECUTION-TRACKER.md)).
- If a verification gate does not exist yet, say so explicitly and add it to the
  future-gates list.
- **Report outcomes faithfully.** "Built but not gated," "designed but not
  implemented," and "passes tests but has no gate" are said plainly, never
  rounded up to "done."

## Never do this

- **Never invent a decision.** If a design question is open, surface it — do not
  resolve it silently in an implementation. That is exactly how the archived
  drafts drifted.
- Never disable a CI gate to ship. If a gate is wrong, change it by ADR, in the
  open.
- Never copy source, schema, templates, assets, or strings from OrangeScrum or any
  other tracker. This is a clean-room implementation
  ([docs/09-REPOSITORY-AND-CONTRIBUTION.md](docs/09-REPOSITORY-AND-CONTRIBUTION.md)).
- Never write `unsafe`. `unsafe_code = "forbid"` at the workspace root; an
  exception requires an ADR.
- Never log customer content — task titles, descriptions, comment bodies —
  or any credential, at any level
  ([docs/46-OBSERVABILITY-AND-OPERATIONS.md](docs/46-OBSERVABILITY-AND-OPERATIONS.md)).
- Never use the words "lossless", "seamless", "simply", or "just" in a document.
  Each hides an unverified claim.

## Current state

**Phase 0 closed (2026-08-08). Phase 1 — usable core — is well under way.**

`docs/` is finished for Phases 0–4 (45 numbered documents, 32 Accepted ADRs).

**There is a product, and it runs.** Sign in, create a project, raise a task
with a description, assignee, priority and due date, move it on a board, filter
a list on any column, read a dashboard, upload an attachment. That sentence
replaced "no product functionality exists yet", which stayed here long after it
stopped being true and told every agent reading this contract the opposite of
what the repository contains.

What that means precisely, in this document's own vocabulary, where `Gated` —
not `Built` — is the only word for done:

| Phase | Items | `Gated` | `Built` | `Building` | Not started |
| --- | --- | --- | --- | --- | --- |
| 0 — foundation (`F`) | 16 | 13 | 3 | — | — |
| 1 — core (`C`) | 48 | 11 | 23 | 13 | 1 |

Do not describe this project as "built and gated" as a whole. That is true of 24
rows and [docs/14](docs/14-EXECUTION-TRACKER.md) names every one; anything less
is reported as what it is.

Phase 0's machinery runs on every pull request and is what makes the rest
verifiable: the enforced dependency DAG and architecture lints, the schema with
row-level security proven as the non-superuser role, the deployable image, the
deterministic 2M-task reference corpus, the `EXPLAIN` no-seq-scan gate over all
29 read paths, the axe and real-browser suites, and the ADR-024 bundle budget.

The three Phase 0 rows that are `Built` and not `Gated` each carry the reason
written down. [docs/14](docs/14-EXECUTION-TRACKER.md) §Current state checks the
closure against the exit gate in
[docs/06](docs/06-ROADMAP-AND-DELIVERY.md), condition by condition.

Ten `D-###` decisions carry an explicit `Accepted` ruling, all settled on 2026-08-08. **D-048** is open (base images are
pinned by mutable tag, not digest — found by the threat-model review), and
**D-033**, **D-034**, **D-045** are deliberately deferred. Eight decisions
accepted on 2026-08-08 have not yet had their design notes rewritten; docs/14
names them, and flags the one that is actively misleading until it is —
[docs/25](docs/25-EVENTS-OUTBOX-AND-AUDIT.md) §Dispatch still describes the
design D-038 rejected.

Three decisions are genuinely open and tracked as such in
[docs/14-EXECUTION-TRACKER.md](docs/14-EXECUTION-TRACKER.md): auth protocol
specifics (D-032, Accept at Phase 0), custom-field value storage (D-033, before
Phase 3), and data residency (D-034, before any customer commitment).

Phase 0's exit gate passed and Phase 1 began. The public page at
<https://casualoffice.github.io/TaskForge/> is generated from `site/` and states
the same counts as the table above; if you change what is true, change all of
them together.
