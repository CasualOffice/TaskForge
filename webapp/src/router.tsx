/**
 * The route tree, and the one search parameter every route shares.
 *
 * # Why the open task is a search parameter and not a path
 *
 * docs/42 §Rendering strategy: "Detail opens in a drawer over the board,
 * preserving scroll position and context, with a full-page route retained for
 * deep links and new tabs." A path segment cannot express that — navigating to
 * `/tasks/{id}` unmounts the board underneath, and coming back re-fetches and
 * re-scrolls it. `?task={id}` keeps the view mounted *and* stays a real URL that
 * can be copied, bookmarked, and opened in a new tab.
 *
 * # Why only Reports is lazy
 *
 * docs/42 §What is in the shell names the board, the list, the drawer and the
 * palette as shell, and reports as lazy-always. Splitting a route the user
 * reaches in the first five seconds trades 3 KiB of budget for a spinner on the
 * critical path, which is the wrong direction for a "first usable shell < 2.5 s"
 * target.
 */
import { lazy, Suspense, type ReactElement } from 'react'
import { createRootRoute, createRoute, createRouter } from '@tanstack/react-router'

import { AppFrame } from './shell/AppFrame'
import { BoardView } from './views/BoardView'
import { MyWorkView } from './views/MyWorkView'
import { TaskListView } from './views/TaskListView'

const ReportsView = lazy(() => import('./views/ReportsView'))

/**
 * The search parameters every route understands.
 *
 * # Why a filter lives in the URL and not in component state
 *
 * A filtered view is a *thing a person means* — "urgent bugs in Platform that
 * are overdue" — and the only way to hand that to a colleague, bookmark it, or
 * come back to it after a reload is for it to be the address. Filters kept in
 * `useState` are lost on refresh, cannot be linked, and quietly disagree between
 * the board and the list when a user switches views expecting the same scope.
 *
 * # Why the values are the server's own grammar
 *
 * `priority` here holds `HIGH,URGENT`, and `due` holds `<@today` — the exact
 * spellings `docs/27` §URL form defines, passed through untranslated. A client
 * dialect would be a second grammar to keep in step with the first, and `docs/27`
 * is explicit that there is one AST with two entry points. The visible cost is
 * that the address bar shows `due=%3C%40today`; the benefit is that a filter that
 * works in the UI works in `curl`, and neither can drift from the other.
 */
export interface AppSearch {
  /** The task open in the drawer, if any. */
  readonly task?: string
  /** The project a view is scoped to. Absent means "everything visible". */
  readonly project?: string
  /** Free-text search, passed through to the server's `q` filter. */
  readonly q?: string
  /** Workflow status ids, comma-separated. */
  readonly status?: string
  /** `HIGH,URGENT` — the grammar's `in`. */
  readonly priority?: string
  /** `BUG,INCIDENT`. */
  readonly type?: string
  /** A user id, or `@me`, or empty for the grammar's `is_empty` (unassigned). */
  readonly assignee?: string
  /** A user id or `@me`. Distinct from `assignee`: who raised it, not who works it. */
  readonly reporter?: string
  /** A date clause: `<@today`, `<+7d`, `@today..+7d`. */
  readonly due?: string
}

/** Every parameter above, so validation and clearing cannot drift from the type. */
export const SEARCH_KEYS = [
  'task',
  'project',
  'q',
  'status',
  'priority',
  'type',
  'assignee',
  'reporter',
  'due',
] as const

/**
 * The filter parameters, as distinct from navigation state.
 *
 * `task` is not one — it says which drawer is open, not which rows to fetch — and
 * neither is `project`, which "Clear filters" deliberately preserves: dropping
 * the project would empty the board and unset the workflow, which is not what
 * anyone means by clearing a filter.
 */
export const FILTER_KEYS = [
  'q',
  'status',
  'priority',
  'type',
  'assignee',
  'reporter',
  'due',
] as const

/**
 * Validate rather than cast.
 *
 * TanStack Router hands the raw parsed query string here, which is attacker
 * controlled: a `task` that is an array or an object would reach a template
 * literal and produce a request to a path nobody wrote. Only strings survive.
 *
 * The empty string is kept for `assignee` alone, because `assignee=` is how the
 * grammar spells "unassigned" (`Operator::IsEmpty`) — dropping it the way every
 * other empty value is dropped would silently turn "show me unassigned work"
 * into "show me everything".
 */
function validateSearch(raw: Record<string, unknown>): AppSearch {
  const out: Record<string, string> = {}
  for (const key of SEARCH_KEYS) {
    const value = raw[key]
    if (typeof value !== 'string') continue
    if (value === '' && key !== 'assignee') continue
    out[key] = value
  }
  return out as AppSearch
}

const rootRoute = createRootRoute({ component: AppFrame, validateSearch })

const listRoute = createRoute({ getParentRoute: () => rootRoute, path: '/', component: TaskListView })
const boardRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: '/board',
  component: BoardView,
})
const myWorkRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: '/my-work',
  component: MyWorkView,
})
const reportsRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: '/reports',
  component: function ReportsRoute(): ReactElement {
    return (
      <Suspense fallback={<p className="empty">Loading reports…</p>}>
        <ReportsView />
      </Suspense>
    )
  },
})

/**
 * Exported so a test can build its own router over the same tree.
 *
 * `createRouter` produces a singleton the module augmentation below types the
 * whole app against; a test that reused it would share history between cases and
 * see the previous test's route.
 */
export const routeTree = rootRoute.addChildren([listRoute, boardRoute, myWorkRoute, reportsRoute])

export const router = createRouter({ routeTree })

declare module '@tanstack/react-router' {
  interface Register {
    router: typeof router
  }
}
