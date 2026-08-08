# 09 — Repository & Contribution

How the repository is organized, how changes get in, and the legal constraints
that apply to every contribution.

## Layout

Full crate detail in [19](19-WORKSPACE-SCAFFOLD-DESIGN.md).

```
tasks/
├── docs/            the design record — source of truth
├── crates/          the Rust workspace
├── migrations/      versioned SQL, forward-only
├── webapp/          React client
├── tools/           seed corpus, load test
├── fixtures/        golden permission matrices, event samples
├── benchmarks/      committed baselines per named environment
├── fuzz/            separate workspace (pinned nightly)
├── AGENTS.md        the contract for coding agents
├── CONTRIBUTING.md  human contributor guide
├── SECURITY.md      coordinated disclosure
└── LICENSE          Apache-2.0
```

## Branching & commits

- `main` is always releasable and protected. All gates must pass
  ([15](15-CI-AND-RELEASE-GATES.md)).
- Branches: `feat/`, `fix/`, `docs/`, `refactor/`, `chore/`.
- **Conventional Commits**, with the tracker ID in the footer:

```
feat(authz): add constraint evaluation to the resolver

Implements assignee_is_actor and reporter_is_actor per docs/04.
Unconstrained grants take precedence over constrained ones.

Refs: C-003
ADR: ADR-004
```

The tracker reference is not decoration — it is how a change is traced back to
the design that justified it, months later.

## Pull requests

**One coherent capability per PR.** A PR that adds a feature *and* refactors a
module is two PRs; the refactor hides the feature's real diff.

Required in the description:

- What and why, linked to the design note.
- Tracker ID.
- ADR reference, if a trigger fired.
- Which acceptance gates now cover it.
- What was *not* done, and why.

That last item is the one most often skipped and most often needed. A PR that
implements four of five acceptance criteria is fine; a PR that silently implements
four of five is not.

### Review checklist

- Does it match the design note? If it diverges, is the note updated in this PR?
- Is every new query path indexed and asserted
  ([26](26-SEARCH-INDEXING-AND-QUERY.md))?
- Is every mutation authorized and tenant-scoped?
- Does it write activity/audit/outbox in the same transaction?
- Are new error codes registered ([20](20-ERROR-CODE-REGISTRY.md))?
- Are limits bounded ([21](21-API-LIMITS-AND-QUOTAS.md))?
- Do tests cover the failure modes, not only the happy path?
- Does it add a user-facing noun? → ADR required.

## Definition of done

A change is done when it is **`Gated`**, not `Built`
([14](14-EXECUTION-TRACKER.md)):

1. Code merged, all CI gates green.
2. Tests cover the behaviour **and its failure modes**.
3. An acceptance gate protects it from regression.
4. Design note, ADR register, support matrix, and tracker updated.
5. Error codes, limits, and filter fields registered where applicable.

## Clean-room constraint

**Binding on every contribution.**

TaskForge is an original implementation licensed Apache-2.0. It **must not**
contain source code, database schemas, templates, assets, strings, or
documentation copied or adapted from OrangeScrum or any other tracker.

What is permitted:

- Studying **published behaviour and public documentation** of other products,
  recorded with sources and dates ([12](12-COMPETITIVE-ANALYSIS.md)).
- Implementing well-known patterns (RBAC, transactional outbox, cursor
  pagination) from general engineering knowledge.
- Interoperating with published formats and public APIs.

What is not:

- Reading another tracker's source and writing the equivalent here.
- Copying a schema, a permission table, a UI template, or an icon set.
- Porting code through a translator, an LLM, or a paraphrase.

Contributors acknowledge this in `CONTRIBUTING.md`. If you have worked on a
competing product, say so before contributing to the corresponding area — this
protects you as much as the project.

## Licensing

- **Apache-2.0**, including the patent grant.
- Dependencies must be Apache-2.0-compatible; `cargo-deny` fails the build
  otherwise. **No copyleft dependencies**, however convenient.
- Contributions are under Apache-2.0 by the DCO (`Signed-off-by`). No CLA.
- Third-party attributions maintained; SBOM published per release.

## Security disclosure

In `SECURITY.md`. Summary: report privately, never in a public issue.
Acknowledgement within 48 hours, assessment within 5 business days, coordinated
disclosure. Security fixes ship out of band and bypass the normal release
cadence — but not the CI gates.

## Working with coding agents

`AGENTS.md` is the entry contract for any agent working in this repository. The
same rules apply as to humans, plus:

- **Read `docs/` first.** Start at [00](00-README.md); read the notes and ADRs
  touching the area.
- **Design before code.** If no design note covers the change, write one.
- **Never invent a decision.** If a design question is open, surface it rather
  than resolving it silently in code. An undocumented decision made in an
  implementation is exactly how the archived drafts drifted.
- **Report outcomes faithfully.** "Built but not gated" is said plainly, not
  rounded up to "done."
- Small, reviewable increments — one capability per PR, same as anyone else.
