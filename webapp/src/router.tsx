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

/** The search parameters every route understands. */
export interface AppSearch {
  /** The task open in the drawer, if any. */
  readonly task?: string
  /** The project a view is scoped to. Absent means "everything visible". */
  readonly project?: string
  /** Free-text search, passed through to the server's `q` filter. */
  readonly q?: string
}

/**
 * Validate rather than cast.
 *
 * TanStack Router hands the raw parsed query string here, which is attacker
 * controlled: a `task` that is an array or an object would reach a template
 * literal and produce a request to a path nobody wrote. Only strings survive.
 */
function validateSearch(raw: Record<string, unknown>): AppSearch {
  const text = (value: unknown): string | undefined =>
    typeof value === 'string' && value !== '' ? value : undefined
  return {
    ...(text(raw['task']) === undefined ? {} : { task: text(raw['task']) }),
    ...(text(raw['project']) === undefined ? {} : { project: text(raw['project']) }),
    ...(text(raw['q']) === undefined ? {} : { q: text(raw['q']) }),
  }
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
