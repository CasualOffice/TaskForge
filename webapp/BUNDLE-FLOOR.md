# Bundle floor measurement — ADR-024

**Measured: 2026-08-08.** Task F-012. This is the measurement ADR-024 promised
and [docs/42-FRONTEND-ARCHITECTURE.md](../docs/42-FRONTEND-ARCHITECTURE.md)
§"The bundle budget" made a precondition of the 200 KB number.

---

## Verdict

**The 200 KB budget survives, with 87 KiB of gzip headroom — but it is a
working budget, not a comfortable one, and the headroom is smaller than the
number suggests because the design system and every line of product code are
still outside this measurement.**

| | gzip | brotli |
| --- | ---: | ---: |
| **Measured initial chunk (dependency floor)** | **113.2 KiB** (115,960 B) | **98.9 KiB** (101,227 B) |
| ADR-024 budget | 200 KiB | 200 KiB |
| **Headroom for design system + all product code** | **86.8 KiB (43.4%)** | 101.1 KiB (50.5%) |

The dependency set alone consumes **57% of the gzip budget** before the shell
renders anything a user would recognise as TaskForge.

### Three findings that change how the ADR should be worded

1. **The unit is undefined and it is worth 2.3% of the budget.** "200 KB" is
   200 KiB (204,800 B) or 200 kB (200,000 B) depending on the reader — a 4.7 KiB
   spread, comparable to the entire `@tanstack/react-virtual` contribution
   (6.7 KiB). The gate in `scripts/size-check.mjs` uses **KiB** and prints the
   kB equivalent; ADR-024 should say KiB explicitly.
2. **The floor moves 4.4% with a bundler major, with no code change.** Vite 8
   (rolldown 1.2.3) produces 113.2 KiB gzip; Vite 7 (rollup 4.62.4) produces
   118.2 KiB gzip from identical source. A gate that fails a PR must not be
   this sensitive to an unrelated dependency bump — pin the bundler, or expect
   a Vite upgrade to spend 5 KiB of budget.
3. **`@tanstack/react-router` is the expensive surprise.** At 24.7 KiB gzip
   marginal it costs more than TanStack Query (9.6), TanStack Virtual (6.7), or
   both dnd-kit packages (13.8) individually — 2.6x Query, and 22% of the whole
   floor. It is the second-largest item in the bundle after `react-dom`. Worth
   knowing before route-typing makes it unremovable.

---

## What was measured

The smallest honest approximation of the authenticated shell described in
docs/42: a two-route TanStack Router tree, a `QueryClient` with one real query,
a `useVirtualizer` list over 2,000 rows, and a dnd-kit sortable list with both
the pointer and the keyboard sensor (docs/42 §Accessibility makes the keyboard
sensor mandatory, so a pointer-only board would have measured smaller than
anything this product is allowed to ship). A third route is `React.lazy`-split
so the report can prove initial and lazy bytes are actually separated.

**No product features, no design system, no styling.** Every dependency is
genuinely reached at runtime; a tree-shaken import would have measured nothing.

Source: `src/main.tsx`, `src/router.tsx`, `src/routes/`.

### Exact versions measured

Runtime dependencies — the docs/42 committed set, plus what they drag in:

| Package | Version | Direct? |
| --- | --- | --- |
| `react` | 19.2.8 | direct |
| `react-dom` | 19.2.8 | direct |
| `scheduler` | 0.27.0 | via react-dom |
| `@tanstack/react-query` | 5.101.4 | direct |
| `@tanstack/query-core` | 5.101.4 | via react-query |
| `@tanstack/react-router` | 1.170.23 | direct |
| `@tanstack/router-core` | 1.171.19 | via react-router |
| `@tanstack/history` | 1.162.1 | via react-router |
| `@tanstack/react-store` | 0.9.3 | via react-router |
| `@tanstack/store` | 0.9.3 | via react-store |
| `isbot` | 5.2.1 | via react-router (**tree-shaken out**, 0 bytes shipped) |
| `@tanstack/react-virtual` | 3.14.9 | direct |
| `@tanstack/virtual-core` | 3.17.7 | via react-virtual |
| `@dnd-kit/core` | 6.3.1 | direct |
| `@dnd-kit/sortable` | 10.0.0 | direct |
| `@dnd-kit/utilities` | 3.2.2 | via dnd-kit (unavoidable — sortable imports it) |
| `@dnd-kit/accessibility` | 3.1.1 | via dnd-kit/core |
| `use-sync-external-store` | 1.6.0 | via TanStack |
| `tslib` | 2.8.1 | via dnd-kit |

Build chain: `vite` 8.2.1 (rolldown 1.2.3), `@vitejs/plugin-react` 6.0.5,
`rollup-plugin-visualizer` 7.0.1, `typescript` 5.9.3, Node 22.18.0, macOS
(darwin 25.3.0, arm64). Build target `es2022`, per docs/18 §Browsers. Minified
by the bundler's default (Oxc for rolldown / esbuild for rollup), no manual
chunking, no sourcemaps in the counted output.

### How the numbers are produced

- **Initial chunk** = the entry chunk plus every chunk reachable from it through
  **static** imports, plus any CSS those chunks import. Chunks reachable only
  through `import()` are lazy and reported separately, never counted
  (`vite.config.ts`, `initialChunkNames`).
- **Compression** is gzip level 9 and brotli quality 11 — what a CDN serves
  pre-compressed. Each file is compressed **separately** and the sizes are
  summed, because that is what HTTP does; compressing a concatenation would
  report a smaller, fictional number.
- The build emits `dist/bundle-report.json`; `scripts/size-check.mjs` reads it
  and is the CI gate. They are separate processes on purpose — the gate can run
  against a downloaded artifact and cannot be defeated by a build-time flag.

---

## Measured result (committed configuration: Vite 8.2.1 / rolldown)

```
initial JS:      113.2 KiB gzip     98.9 KiB brotli   (362.6 KiB raw, 1 file)
initial CSS:       0.0 KiB gzip      0.0 KiB brotli   (0 files — no design system yet)
INITIAL TOTAL:   113.2 KiB gzip     98.9 KiB brotli   = 116.0 kB (1000-byte) gzip
lazy chunks:       0.2 KiB gzip      0.1 KiB brotli   (1 file, NOT counted)
```

Raw bytes for reproducibility: initial 371,288 raw / 115,960 gzip / 101,227
brotli. Lazy 177 raw / 164 gzip / 122 brotli.

### Cross-check: Vite 7.1.9 / rollup 4.62.4, identical source

```
INITIAL TOTAL:   118.2 KiB gzip    103.3 KiB brotli
```

Raw bytes: 379,703 raw / 121,045 gzip / 105,748 brotli. **+5.0 KiB gzip (+4.4%)
versus rolldown.** Both readings are within budget; the point is that the
gate's headline number depends on a build tool the team will upgrade.

---

## Per-dependency contribution

Built as a ladder — five entry points, each adding exactly one library to the
one below it (`src/floor/step-*.tsx`, driven by `scripts/measure-deps.mjs`).
The costs below are **marginal, not standalone**: shared code is attributed to
whichever step pulls it in first, so the column sums to the floor but is
**order-dependent**. Adding `@tanstack/react-router` first would move some of
its cost onto whatever came after. Stated plainly because the alternative —
measuring each library alone and summing — would over-count by roughly the size
of React.

Vite 8.2.1 / rolldown:

| Step adds | marginal gzip | marginal brotli | cumulative gzip | cumulative brotli | share of gzip floor |
| --- | ---: | ---: | ---: | ---: | ---: |
| `react` + `react-dom` (+ `scheduler`) | 57.6 KiB | 49.6 KiB | 57.6 KiB | 49.6 KiB | **50.8%** |
| `@tanstack/react-query` (+ `query-core`) | 9.6 KiB | 8.8 KiB | 67.2 KiB | 58.4 KiB | 8.5% |
| `@tanstack/react-router` (+ `router-core`, `history`, `store`) | **24.7 KiB** | 21.9 KiB | 91.9 KiB | 80.3 KiB | **21.8%** |
| `@tanstack/react-virtual` (+ `virtual-core`) | 6.7 KiB | 5.9 KiB | 98.6 KiB | 86.1 KiB | 5.9% |
| `@dnd-kit/core` + `@dnd-kit/sortable` (+ `utilities`, `accessibility`, `tslib`) | 13.8 KiB | 12.1 KiB | 112.3 KiB | 98.2 KiB | 12.2% |
| *(harness plumbing: third route, `Suspense`, nav)* | 0.9 KiB | 0.6 KiB | **113.2 KiB** | **98.9 KiB** | 0.8% |

Same ladder under Vite 7 / rollup, for the sensitivity check: 59.1 / 9.9 / 26.4
/ 7.1 / 14.8 KiB gzip marginal, cumulative 117.3 KiB, full shell 118.2 KiB. The
ordering and proportions hold; only the absolute number moves.

**Three of the five dependency groups clear the ≥ 10 KiB gzip
written-justification threshold** docs/42 sets for new dependencies (React,
Router, dnd-kit; Query falls under at 9.6 KiB, Virtual well under at 6.7).
They are already committed by docs/42, so no justification is owed — but the
shell starts with three dependencies that would each require a written PR
argument if proposed today.

### Where the bytes sit inside the single chunk

From `rollup-plugin-visualizer` (`dist/stats.html`, a CI artifact per docs/42).
These are **pre-minification module sizes**, so they sum to more than the
shipped bundle and are useful for *proportion only* — the ladder above is the
authoritative KiB attribution.

| Package | share of rendered module bytes |
| --- | ---: |
| `react-dom` | 51.8% |
| `@tanstack/router-core` | 13.1% |
| `@dnd-kit/core` | 8.7% |
| `@tanstack/query-core` | 7.1% |
| `@tanstack/react-router` | 4.5% |
| `@tanstack/virtual-core` | 4.4% |
| `react` | 1.9% |
| everything else (12 packages + app source) | 8.6% |

`react-dom` is half the floor and is not reducible without an ADR that changes
the framework. TanStack Router's two packages together are ~17.6% — the largest
reducible item.

---

## What is NOT in this 113.2 KiB — read this before trusting the headroom

The measured floor is the **dependency** floor. Everything below is real
initial-bundle weight that this harness does not contain:

1. **`@schnsrw/design-system`** — tokens and primitives, consumed per AGENTS.md
   and docs/42. Not resolvable in this checkout, so **not measured at all**. A
   primitives set (buttons, inputs, menus, dialogs, toasts, focus management)
   plausibly costs 15–40 KiB gzip. This is the single largest unknown.
2. **CSS: measured as 0 KiB.** The harness has no stylesheet. Real shell CSS is
   initial and render-blocking.
3. **Material Symbols icons.** ADR-024 budgets *JS*, so an icon font sits
   outside the number while still competing for the < 2.5 s first-usable-shell
   target in docs/42 §Performance targets. Worth naming in the ADR so it is not
   discovered later as a loophole.
4. **All product code in the shell** per docs/42 §What is in the shell: app
   frame, navigation, auth, task list, board, task drawer, command palette,
   filter builder — plus the generated API client and types, the optimistic
   mutation layer, the permission hook, the SSE client and its
   `BroadcastChannel` sharing, the IndexedDB draft cache, and the retry queue.
5. **TanStack Router devtools** must stay dev-only. If they reach a production
   build they add tens of KiB and the gate is the only thing that will notice.
6. **`isbot` shipped 0 bytes here** because the SSR path is unreached. Adopting
   SSR or streaming would change that and several other router costs.
7. **No i18n or locale data.** docs/42 already scopes date-picker locale data to
   lazy; nothing else is budgeted.

**Realistic reading:** 86.8 KiB gzip is left for items 1–4. That is enough — the
shell described in docs/42 is not large — but it is not generous, and a single
careless dependency (a chart library, a second icon set, a date library that is
not `Intl`) consumes a quarter of it.

---

## Recommendation to ADR-024

**Keep 200 as the number; amend three things.** The budget was set as a guess
and the guess held, which is the outcome worth recording plainly.

1. **State the unit: 200 KiB (204,800 bytes), gzip, initial chunk only.** Gzip
   rather than brotli as the gating metric, because it is the worse case and not
   every edge serves brotli. Brotli is reported for information.
2. **Define "initial" in the ADR** the way the gate defines it: entry chunk plus
   its static-import closure plus imported CSS; dynamic-import chunks excluded.
   Without that sentence, "the initial chunk" is a matter of opinion at review
   time.
3. **Pin or record the bundler.** A Vite major moves the measurement 4.4% with
   no source change. Either pin the version in CI or record the bundler version
   in the gate output so a jump is attributable.

**Re-measure and revisit when `@schnsrw/design-system` is wired in** — that is
the one input that could still break 200 KiB, and it is the only item in this
document that could not be measured at all.

---

## Reproducing

```sh
cd webapp
pnpm install --frozen-lockfile
pnpm build            # writes dist/bundle-report.json and dist/stats.html
pnpm size-check       # the CI gate; exits 1 over budget, 2 on a missing report
pnpm measure:deps     # rebuilds the per-dependency ladder into dist-floor/
```

Gate options: `--budget-kib <n>` (or `TASKFORGE_BUNDLE_BUDGET_KIB`), `--metric
gzip|brotli`, `--report <path>`.

**Verified on 2026-08-08:** `pnpm install`, `pnpm typecheck`, `pnpm build`,
`pnpm size-check`, and `pnpm measure:deps` were all run; the numbers above are
their output. The failure path was exercised (`--budget-kib 100` exits 1) and
the missing-report path exits 2.

**Not verified:** the app has never been run in a browser, and there are no
tests. It exists to be measured, not to be used. `pnpm dev` should work but was
not exercised.

## Gate status

**Built, not gated.** `scripts/size-check.mjs` is the mechanism and its exit
codes are verified, but nothing in `.github/workflows/ci.yml` calls it yet —
wiring it is the integrator's step, per docs/15-CI-AND-RELEASE-GATES.md. Until
that lands, this measurement protects nothing.
