# CLAUDE.md

This repository is governed by [AGENTS.md](AGENTS.md). Read it first — it is the
full contract. This file only restates the essentials.

- **Design first, then execute.** The permission model, the outbox, the extension
  registry, and the index contract are fixed before code; do not defer design or
  plan for do-overs.
- **Discuss and finalize** substantial designs before implementing.
- **Track everything.** Every unit of work has a row in
  [docs/14-EXECUTION-TRACKER.md](docs/14-EXECUTION-TRACKER.md), created when it
  starts and updated as its status moves. No untracked work.
- **Production-grade baseline.** Authority, tenant isolation, and traceability
  come before performance, and performance is a designed-in gated target
  (p95 read < 150 ms at 2M tasks, no sequential scans, ≤ 200 KB client shell),
  not an afterthought.
- **Make the wrong thing impossible, not discouraged.** Prefer a compile error or
  a database grant to a rule in a document.
- **"Done" means `Gated`** — merged, tested, *and* protected by an acceptance
  gate. Report anything less faithfully.
- **Never invent a decision.** Surface open design questions; do not resolve them
  silently in code.

Current state: **Phase 1 — usable core, well under way.** Phase 0 closed on
2026-08-08. Design record complete (45 numbered docs, 32 Accepted ADRs), and
**there is a working product**: sign in, projects, tasks, board, list,
dashboards, environments, releases, attachments.

Counts, in this repository's own vocabulary, where `Gated` — not `Built` — means
done: Phase 0 is 13 `Gated` / 3 `Built` of 16; Phase 1 is 12 `Gated` /
28 `Built` / 9 `Building` / 1 not started, of 50. Never describe the project as
"built and gated" as a whole. See AGENTS.md §"Current state", which carries the
same table, and docs/14 for every row by name.
