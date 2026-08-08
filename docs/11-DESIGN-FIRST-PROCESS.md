# 11 — Design-First Process

TaskForge is designed before it is built (ADR-002). Not ceremony: the permission
model, the event model, the index contract, and the extension registry are all
load-bearing for everything above them, and a wrong decision at that level is
expensive in a way a wrong handler is not.

The evidence that this matters is in `docs/_archive/`: the previous drafts left
nine architectural decisions explicitly open, and the one decision that *was*
made (Rust) never propagated to four of the six documents. Design that is not
written down does not survive contact with a second author.

## The eight steps

1. **Problem definition.** What a user or integrator can do afterward that they
   cannot now. One paragraph.
2. **Research.** How do Jira, Linear, GitHub, Atlassian Forge, Redmine handle it?
   Record the source and the **date checked** — research goes stale
   ([16](16-DOCUMENTATION-MAINTENANCE.md)).
3. **Design note.** A numbered `docs/` note covering the model impact, the layers
   touched, the failure modes, the limits, and the acceptance gates.
4. **Discussion & finalization.** Substantial designs are discussed and marked
   final before implementation. If an ADR trigger fires, the ADR must be
   **Accepted** first.
5. **Tracker update.** A row in [14](14-EXECUTION-TRACKER.md) with a stable ID.
6. **Implementation.** Small, reviewable increments; one capability per PR.
7. **Verification.** Run the gates in [15](15-CI-AND-RELEASE-GATES.md); prove the
   acceptance criteria from step 3.
8. **Documentation.** Update the note, the ADR register, the support matrix, and
   the tracker, so the written design and the code never diverge.

## ADR triggers

Write an ADR (record it in [08](08-ADR-REGISTER.md)) when a decision touches:

- The **permission model** — resolution, scopes, constraints, ceilings.
- **Tenancy** — how isolation is enforced, or any exception to it.
- A **public API surface** — endpoints, payloads, error codes, cursors.
- A **crate boundary** or the dependency DAG ([19](19-WORKSPACE-SCAFFOLD-DESIGN.md)).
- The **database schema** in a way that is not purely additive.
- **Event payloads** or the outbox contract.
- **The filterable/sortable field set**, or an index strategy.
- **Workflow or state semantics** — especially the five states.
- **The extension point registry**, plugin trust, or scopes.
- **Concurrency semantics** — versioning, idempotency, locking.
- A **security control** or a limit that is also a security bound.
- A **dependency choice** (new crate, new infrastructure component).
- A **performance budget** that constrains a lower layer.
- Anything **adding a noun** to the user's vocabulary ([01](01-ORD.md)).

That last trigger is the simplicity contract, made procedural. It is the only one
that fires on a *product* decision rather than a technical one, and it is the one
most likely to be skipped.

## "Held back, not un-designed"

Custom roles ship in Phase 2, plugins in Phase 3, automations in Phase 4 — but all
three are designed now, because each dictates a seam in Phase 1:

- Custom roles ⇒ the Phase 1 resolver must be the final algorithm, with built-in
  roles as ordinary data.
- Plugins ⇒ the extension registry must exist in Phase 1, used by core panels.
- Automations ⇒ the outbox must be written from the first mutation, and the filter
  grammar must be reusable as a condition language.

Any design that would force those Phase 1 layers to be rewritten when the later
phase arrives is **rejected**, not deferred.

## Design note template

```
# NN — Title

## Outcome
What is possible afterward that is not possible now.

## Research (sources + dates checked)

## Design
### Domain / schema impact
### Layers & crates touched
### API surface
### Failure modes & limits
### Security & tenancy implications

## Alternatives considered
Each with the reason it was not chosen. "Nothing else was considered" is a
finding, not an omission to hide.

## Acceptance gates
The tests that prove it. Specific enough that someone else could write them.

## ADRs triggered

## Tracker IDs
```

## Review expectations

A design note is ready when a reviewer can answer, without asking:

- What breaks if this is wrong?
- What is the failure mode under load, under partial failure, under a malicious
  input?
- How is it authorized, and how is it tenant-isolated?
- What is the query, and what index serves it?
- What does it cost the user in concepts they must learn?
- How will we know it works, in CI, without a human?

If the note cannot answer those, it is not finished — and the code written from
it will not answer them either.
