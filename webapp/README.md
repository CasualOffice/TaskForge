# webapp — the TaskForge web client

The client [docs/42-FRONTEND-ARCHITECTURE.md](../docs/42-FRONTEND-ARCHITECTURE.md)
designs: React 19, TanStack Query and Router, TanStack Virtual, dnd-kit, plain
CSS with custom properties. It talks to the real API and to nothing else —
there is no mock, no fixture and no stub anywhere under `src/` outside
`src/floor/`.

This directory began as the bundle-floor measurement rig for
[ADR-024](../docs/08-ADR-REGISTER.md), and that rig is still here and still
runs — see [BUNDLE-FLOOR.md](BUNDLE-FLOOR.md) and `src/floor/`. It moved out of
the way rather than being deleted, for the reason its own module docs give: once
product code sits at `src/api.ts` and `src/routes/`, a ladder that imports those
paths measures product bytes, and the floor stops meaning what its name says.

## Running it

```
scripts/dev-up.sh          # from the repository root: database, API, and this
```

`pnpm dev` on its own serves the client at `:5173` and proxies `/api` to
`$VITE_API_URL` (default `http://127.0.0.1:8080`). **The browser must stay
same-origin.** The session cookie is `HttpOnly` and `SameSite=Lax` and the API
registers no CORS layer, so a client pointed straight at `:8080` from `:5173`
has every authenticated request refused before it arrives. Development therefore
has the same origin shape production does: one origin, `/api` behind it.

## Commands

| Command | What |
| --- | --- |
| `pnpm install --frozen-lockfile` | install |
| `pnpm dev` | dev server with the `/api` proxy |
| `pnpm typecheck` | `tsc --noEmit` |
| `pnpm lint` | ESLint: `jsx-a11y`, `react-hooks`, TypeScript correctness (C-019) |
| `pnpm test` | Vitest: the axe suite, the SSE frame parser, and the boot test |
| `pnpm build` | build; writes `dist/bundle-report.json` and `dist/stats.html` |
| `pnpm size-check` | the ADR-024 gate; exit 1 over budget, exit 2 on a missing report |
| `pnpm measure` | build then gate |
| `pnpm measure:deps` | rebuild the per-dependency ladder into `dist-floor/` |

Gate options: `--budget-kib <n>` (or `TASKFORGE_BUNDLE_BUDGET_KIB`),
`--metric gzip|brotli` (or `TASKFORGE_BUNDLE_METRIC`), `--report <path>`.

All of these run in CI (`bundle-size` and `frontend-a11y`) and in
`scripts/check.sh`.

## How the source is laid out

Split by **reason to change**, which for a client means "which document moves
when this file does" (AGENTS.md §Module size and shape):

| Directory | Changes when |
| --- | --- |
| `api/` | the API contract does — one module per resource, plus `http.ts` (transport) and `problem.ts` (the `docs/20` code registry) |
| `shell/` | the frame does: session, workspace, theme, focus, live region, navigation |
| `views/` | a screen does |
| `drawer/` | the task detail does |
| `palette/` | ⌘K does |
| `tasks/` | task-domain behaviour does: the paged feed, optimistic mutations, presentation |
| `live/` | the SSE contract does |
| `extensions/` | `docs/34`'s registry does |
| `floor/` | never — it is the ADR-024 measurement rig and must not grow product code |

Three of those are worth knowing about before you touch anything:

- **`api/http.ts` is the only place that calls `fetch`.** `docs/05` and `docs/40`
  put four obligations on every browser request — the session cookie, the
  double-submit CSRF token, the workspace header, and `If-Match` /
  `Idempotency-Key` on writes. A call site cannot forget the CSRF header because
  it never sets one.
- **`api/problem.ts` is the only place that decides what a refusal says.** The
  server's own message never reaches the screen; a registry code from `docs/20`
  maps to a sentence and a remedy. There is a test for that.
- **`api/keys.ts` is the only place that spells a cache key,** and every tenant
  key begins with the workspace id, so a workspace switch invalidates by prefix
  and no key can be written that omits the tenant.

## What is not built yet

The web client is ahead of the API in several places and behind it in none. The
authoritative list — ten rows, each with the tracker item that closes it — is in
[docs/14-EXECUTION-TRACKER.md](../docs/14-EXECUTION-TRACKER.md) under
*C-018 and C-019*. In short: assignees are write-only in the API, and relations,
activity and attachments have no read endpoint. Each one renders on screen as a
stated gap rather than as an empty panel, so the product never claims to have
lost a feature it never had.
