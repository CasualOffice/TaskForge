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

Current state: **Phase 0 — foundation.** Design record complete (37 docs, 26
ADRs); workspace scaffolding in progress; no product functionality yet. See
AGENTS.md §"Current state".
