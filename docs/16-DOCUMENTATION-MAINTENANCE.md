# 16 — Documentation Maintenance

`docs/` is the source of truth. Code follows docs. Keeping that true is a
process, not an intention.

## The failure this prevents

It already happened here. The archived drafts split one master document into five
markdown files by **copying sections**. When the backend decision changed from
Java to Rust, the `.docx` was updated and the five copies were not — so six files
described two incompatible architectures, and the wrong one was the one an
engineer would have opened first.

See [`_archive/README.md`](_archive/README.md).

## The three rules

### 1. One owner per fact

Every fact lives in exactly one document. Other documents **link** to it; they do
not restate it.

| Fact | Owner |
| --- | --- |
| Permission resolution | [04](04-RBAC-AND-AUTHORIZATION.md) |
| The five states | [23](23-WORKFLOW-AND-STATE-MACHINE.md) |
| Table definitions | [22](22-DATABASE-SCHEMA.md) |
| Indexes | [26](26-SEARCH-INDEXING-AND-QUERY.md) |
| Limits | [21](21-API-LIMITS-AND-QUOTAS.md) |
| Error codes | [20](20-ERROR-CODE-REGISTRY.md) |
| Performance targets | [30](30-PERFORMANCE-AND-CAPACITY-TARGETS.md) |
| Crate boundaries | [19](19-WORKSPACE-SCAFFOLD-DESIGN.md) |
| Vocabulary | [17](17-GLOSSARY.md) |
| Decisions | [08](08-ADR-REGISTER.md) |

A short restatement for readability is allowed **only** when the canonical link
sits beside it. A limit table duplicated into three docs will diverge; the
question is only when.

### 2. Numbers are stable and never reused

A retired document keeps its number with a tombstone; new documents take the next
free number. Cross-references are by number, so a link never rots because
something was renamed.

Ranges, mirroring `opendoc/` and `sheets/`:

- **00–19** — foundation, process, top-level architecture.
- **20–29** — stable contracts (errors, limits, schema, workflow, concurrency,
  events, search, filters, attachments, notifications).
- **30–49** — architecture pillars (performance, tenancy, plugins, automation,
  identity, frontend, observability, deployment).
- **50+** — per-feature design notes, added as phases open.

### 3. Research carries a date

Any claim about another product, a specification, or an ecosystem convention
carries **date checked**. It must be re-verified before being relied on for a
specific decision ([12](12-COMPETITIVE-ANALYSIS.md)).

Undated research is indistinguishable from a guess after six months.

## When code and docs disagree

**The doc is wrong until proven otherwise**, because the doc is the design and
the code is an implementation of it. Three outcomes:

1. The code is a bug → fix the code.
2. The design changed deliberately → update the doc **in the same PR**, and add
   an ADR if a trigger fired.
3. The design was never right → write the correction and an ADR superseding the
   old decision.

What is never acceptable: leaving them disagreeing and remembering which is real.
That knowledge does not survive the author leaving.

## PR requirements

Enforced by [15](15-CI-AND-RELEASE-GATES.md):

- A design change updates its numbered doc **in the same PR** — never "docs to
  follow."
- An ADR-triggering change has an Accepted ADR **before** merge.
- A new error code, filter field, limit, or event type is registered in its owner
  document in the same PR.
- The tracker row is added or moved ([14](14-EXECUTION-TRACKER.md)).
- Internal links resolve.

## Review checklist for a design note

- Does it duplicate a fact another doc owns? → link instead.
- Does it introduce a term? → is it in [17](17-GLOSSARY.md)?
- Does it introduce a user-facing noun? → ADR trigger.
- Are the acceptance gates specific enough for someone else to implement?
- Are alternatives recorded with **why not**? ("None considered" is itself a
  finding.)
- Is every external claim dated?
- Would a new engineer, reading only this, build the right thing?

## Periodic maintenance

| Cadence | Task |
| --- | --- |
| Per PR | owner docs, tracker, links |
| Per phase gate | competitive analysis re-verified; threat model reviewed; support matrix updated; ADR "pending" list triaged |
| Per release | changelog, support matrix, SBOM, public API docs |
| Annually | full read-through for drift; archive superseded notes with tombstones |

## Writing standards

- **Say what is true, not what sounds good.** "Preserved but not modelled" beats
  "fully supported."
- **Banned words**: "simply", "just", "obviously", "seamless", "lossless" — each
  hides an unverified claim. If something is genuinely simple, the sentence
  describing it will be short without saying so.
- **State costs.** Every trade-off has a losing side; name it. The additive-RBAC
  cost in [04](04-RBAC-AND-AUTHORIZATION.md) is the model for this.
- **Prefer a table to a paragraph** for anything enumerable.
- **Show the query, the payload, the DDL.** Concrete beats descriptive.
- **Explain why, not only what.** The *what* is recoverable from the code; the
  *why* is not, and it is the thing a future reader actually needs.
