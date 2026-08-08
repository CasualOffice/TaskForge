# webapp — bundle-floor harness

**This is not the TaskForge frontend.** It is the measurement rig for
[ADR-024](../docs/08-ADR-REGISTER.md) — the smallest honest program that
genuinely uses every dependency
[docs/42-FRONTEND-ARCHITECTURE.md](../docs/42-FRONTEND-ARCHITECTURE.md) commits
to, so the 200 KB bundle budget could be checked against a real number instead
of an estimate.

**The result is in [BUNDLE-FLOOR.md](BUNDLE-FLOOR.md).** Read that first.

When the real client is built, it grows from this directory — the Vite config,
the size gate, and the `dist/stats.html` artifact are the parts meant to
survive. `src/floor/` and the placeholder routes are not.

## Commands

| Command | What |
| --- | --- |
| `pnpm install --frozen-lockfile` | install |
| `pnpm build` | build; writes `dist/bundle-report.json` and `dist/stats.html` |
| `pnpm size-check` | the ADR-024 gate; exit 1 over budget, exit 2 on a missing report |
| `pnpm measure` | build then gate |
| `pnpm measure:deps` | rebuild the per-dependency ladder into `dist-floor/` |
| `pnpm typecheck` | `tsc --noEmit` |

Gate options: `--budget-kib <n>` (or `TASKFORGE_BUNDLE_BUDGET_KIB`),
`--metric gzip|brotli` (or `TASKFORGE_BUNDLE_METRIC`), `--report <path>`.

## For the CI integrator

Nothing here runs in CI yet. The gate is `pnpm --dir webapp build && pnpm --dir
webapp size-check`, and `webapp/dist/stats.html` is the per-PR artifact
docs/42 asks for. Adding it belongs in
[docs/15-CI-AND-RELEASE-GATES.md](../docs/15-CI-AND-RELEASE-GATES.md); until
then this measurement protects nothing.

## Deliberately absent

No tests, no linter, no formatter config, no design system, no styling, no
application behaviour. Adding any of them would change the number this
directory exists to report. The real client will need all of them.
