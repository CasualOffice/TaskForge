# 42 — Frontend Architecture

The client is a **thin interaction layer**, not a second application server.
Business rules, authorization, filtering, sorting, and workflow validation are
server concerns. The client's job is to render fast, mutate optimistically, and
stay out of the way.

An early visual concept for the shell, board, drawer, and design-system
primitives is kept at [`assets/ui-concept-board.png`](assets/ui-concept-board.png).
It is **illustrative only** — the authoritative tokens and primitives come from
`@schnsrw/design-system`, shared with Casual Sheets and Casual Editor.

> **This document says how the client is built, not what it is for.**
> Which surfaces exist, what question each answers, and what belongs above the
> fold are in [44](44-PRODUCT-RESEARCH-AND-SURFACE-BRIEFS.md). That document was
> written after the fact, and its first section explains what a product looks
> like when it is missing: a screen per endpoint. Read it before adding a view.

## Stack

| Concern | Choice | Why |
| --- | --- | --- |
| Framework | React 19 + TypeScript | suite consistency; the ecosystem for the primitives we need |
| Build | Vite | fast, first-class code splitting, honest bundle analysis |
| Server state | TanStack Query | caching, invalidation, optimistic mutation, retry — the whole problem, solved |
| Routing | TanStack Router | typed routes, route-level code splitting, typed search params |
| Local state | `useState` / `useReducer` / context | there is no global store — see below |
| Drag & drop | dnd-kit | accessible, keyboard-operable, lightweight |
| Lists | TanStack Virtual | virtualization for boards, lists, activity |
| Styling | CSS + custom properties, via `@schnsrw/design-system` | shared with Casual Sheets/Editor; no runtime CSS-in-JS |
| Rich text | lazy-loaded editor, on edit only | never in the initial bundle |
| Testing | Vitest + Testing Library + Playwright | unit → integration → real browser |

**No Redux, Zustand, MobX, or global store.** Nearly all state in a tracker is
*server* state, and TanStack Query already owns it — caching, staleness,
invalidation, and rollback. A parallel global store becomes a second, divergent
copy of the truth, and the reconciliation bugs that follow are the hardest class
of frontend bug to diagnose. Genuinely local state (a drawer being open, a draft
being typed) stays in the component that owns it.

## The bundle budget (ADR-024)

**Target: ≤ 200 KB compressed JS for the authenticated shell**, excluding
lazy chunks. Enforced in CI ([15](15-CI-AND-RELEASE-GATES.md)); exceeding it
fails the build.

**Measured, and it holds.** React + TanStack Query + Router + dnd-kit + Virtual
is a substantial fraction of that before a line of product code, so the budget
was provisional until Phase 0 measured it. It has been:
[`webapp/BUNDLE-FLOOR.md`](../webapp/BUNDLE-FLOOR.md) (tracker F-012) puts the
dependency floor at **113.2 KiB gzip — 57% of the budget**, leaving 86.8 KiB for
the design system, all CSS, and every line of product code.

Three things that measurement changed, and which the gate now reflects:

- The unit is **KiB (204,800 bytes), gzip, initial chunk only** — "200 KB" was
  ambiguous by 4.7 KiB, which is more than the whole `@tanstack/react-virtual`
  contribution.
- "Initial" means the entry chunk plus its static-import closure plus imported
  CSS; `import()`ed chunks are excluded.
- The floor moves **4.4% on a bundler major with no source change**, so the
  lockfile is frozen in the gate.

`@schnsrw/design-system` was **not** resolvable in the measured checkout and is
the one input that could still break 200 KiB. Re-measure when it is wired in. If
the floor is genuinely higher then, the number is raised by a superseding ADR
with the measurement attached — it is never quietly exceeded, and the gate is
never disabled.

The gate exists precisely because bundles do not regress in one bad commit; they
regress 4 KB at a time over a year, with every individual step defensible.

### What is in the shell

App frame, navigation, auth, the task list and board, the task drawer, the command
palette, the filter builder, and the design system primitives actually used.

### What is not — lazy, always

Calendar · timeline/Gantt · reports and charts · all admin and settings screens ·
the rich text editor · the workflow editor · the permission simulator · every
plugin UI · date-picker locale data beyond the active locale.

**Rules that keep it that way:**
- One icon library (Material Symbols, from the design system). Not two.
- One date library. `Intl` where it suffices.
- No moment, no lodash, no full component suite.
- Every dependency ≥ 10 KB gzipped needs a written justification in the PR.
- `rollup-plugin-visualizer` output is a CI artifact on every PR, so growth is
  visible at review time rather than at release time.

## Performance targets

Measured on a mid-tier laptop over throttled broadband (Fast 3G-equivalent RTT),
p75 unless stated ([30](30-PERFORMANCE-AND-CAPACITY-TARGETS.md)):

| Metric | Target |
| --- | --- |
| First usable shell | < 2.5 s |
| Interaction latency (local UI) | < 100 ms |
| Long tasks during board interaction | none > 200 ms |
| Drag frame rate | 60 fps where hardware permits |
| Board with 500 cards | virtualized, no jank |
| Route transition (lazy chunk) | < 300 ms |

## Rendering strategy

**Virtualize everything unbounded.** Boards, lists, activity streams, and comment
threads render only the visible window plus a small overscan. A 2,000-card board
must not mount 2,000 components — and "our biggest customer has 2,000 cards" is
not a hypothetical.

**Server-side everything expensive.** Filtering, sorting, searching, and
pagination are server operations ([26](26-SEARCH-INDEXING-AND-QUERY.md)). The
client never downloads a project to filter it locally — that is the design
mistake that makes a tracker feel fine in development and unusable at a real
customer.

**Detail opens in a drawer** over the board, preserving scroll position and
context, with a full-page route retained for deep links and new tabs.

## Optimistic mutation

```ts
useMutation({
  onMutate: async (vars) => {
    await qc.cancelQueries({ queryKey: taskKey(vars.id) })
    const prev = qc.getQueryData(taskKey(vars.id))
    qc.setQueryData(taskKey(vars.id), optimistic(prev, vars))
    return { prev }                                  // rollback token
  },
  onError: (err, vars, ctx) => {
    qc.setQueryData(taskKey(vars.id), ctx.prev)      // roll back
    if (err.status === 409) resolveConflict(err)     // doc 24
    preserveUserInput(vars)                          // never discard typing
  },
  onSettled: (_d, _e, vars) => qc.invalidateQueries({ queryKey: taskKey(vars.id) }),
})
```

Three rules that are not optional:

1. **Every optimistic update carries a rollback token.** No exceptions.
2. **`409` is handled, not thrown at the user.** Non-overlapping conflicts retry
   automatically; overlapping ones show a diff ([24](24-CONCURRENCY-AND-IDEMPOTENCY.md)).
3. **Failure never discards user input.** A failed comment keeps its text in the
   draft cache. This is the difference between a blip and a betrayal.

## Live updates

One SSE connection per workspace, shared across tabs via `BroadcastChannel` — N
tabs must not mean N streams. Incoming events invalidate the relevant query keys
rather than patching the cache directly; TanStack Query then refetches only what
is mounted. Events are already coalesced server-side over 100 ms
([05](05-API-SPEC.md)).

On reconnect, `Last-Event-ID` replays the gap. If the gap is too large, the client
refetches wholesale rather than accepting a partial history it would treat as
complete.

## Permissions in the UI

```ts
const { can } = usePermissions(projectId)   // from GET /permissions/effective
{can('task.close') && <Button>Close</Button>}
```

Hiding a control is **presentation, never security**. The server re-authorizes
every mutation ([04](04-RBAC-AND-AUTHORIZATION.md)). The permission set is cached
per project and invalidated by the `authz_epoch` bump arriving over SSE, so a
revoked permission disappears from the UI within a second rather than at next
reload.

## Progressive disclosure

The create form asks for a **title**. Project and status default; assignee is one
optional click. Description, dates, tags, environment, milestone, and custom
fields appear on request or once the workspace enables them.

This is the product principle from [01](01-ORD.md) rendered as a form, and it is
the single highest-leverage decision in the UI. Every tracker that opens with
fourteen fields trains its users to paste "TODO" into half of them.

## Command palette

`⌘K` handles create, navigate, assign, transition, search, and plugin-contributed
commands ([34](34-PLUGIN-AND-EXTENSION-ARCHITECTURE.md)). It is why permanent
navigation can stay at seven items — new capability adds a command, not a nav
entry. It also means plugins have somewhere to put an action that is not a new
button on an already-crowded toolbar.

## Accessibility

**WCAG 2.2 level AA** is the standard — the old drafts said "accessibility gates"
without naming one, which is unenforceable.

- Keyboard-operable everything, including drag and drop (dnd-kit's keyboard
  sensor is a requirement, not a nice-to-have).
- Visible focus, logical order, no keyboard traps in drawer or palette.
- Semantic HTML first; ARIA only where semantics run out.
- Live regions announce optimistic outcomes and errors.
- 4.5:1 contrast, verified in light and dark from design system tokens.
- `prefers-reduced-motion` respected.
- Automated axe checks in CI, plus a manual keyboard-only pass per release on the
  core flows. Automation catches perhaps a third of real issues; the manual pass
  is where the rest are found.

## Offline

Not offline-first, deliberately ([01](01-ORD.md)). What ships:

- A **draft cache** in IndexedDB for comment and description text.
- A **retry queue** for failed mutations, with a visible pending indicator.
- Clear degraded state when the stream drops.

Not shipping: a sync engine, conflict-free local replicas, or full offline reads.
Those are a large permanent complexity cost for a use case a web tracker rarely
has, and reversing that decision later is easy — reversing the opposite is not.

## Testing

| Level | What |
| --- | --- |
| Unit (Vitest) | filter builder, cursor handling, optimistic reducers, permission hooks |
| Component (Testing Library) | drawer, board, forms — **by role and label**, never by test id |
| Integration (MSW) | full flows against a mocked API honouring the real OpenAPI schema |
| E2E (Playwright) | create → assign → transition → comment → search, plus a keyboard-only pass |
| Visual | design system primitives, light and dark |
| Budget | bundle size gate on every PR |

MSW mocks are generated from the committed OpenAPI snapshot ([05](05-API-SPEC.md)),
so a server contract change breaks frontend tests immediately rather than in
staging.
