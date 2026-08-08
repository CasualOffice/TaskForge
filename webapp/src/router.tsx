import { lazy, Suspense, type ReactElement } from 'react'
import { createRootRoute, createRoute, createRouter, Link, Outlet } from '@tanstack/react-router'

import { Board } from './routes/Board'
import { TaskList } from './routes/TaskList'

// React.lazy rather than the router's own splitting API: the split boundary is
// what the measurement needs, and this form does not depend on a router
// code-splitting API whose shape has moved between minor versions.
const Reports = lazy(() => import('./routes/Reports'))

function Shell(): ReactElement {
  return (
    <div>
      <nav>
        <Link to="/">Tasks</Link> <Link to="/board">Board</Link> <Link to="/reports">Reports</Link>
      </nav>
      <Outlet />
    </div>
  )
}

const rootRoute = createRootRoute({ component: Shell })

const listRoute = createRoute({ getParentRoute: () => rootRoute, path: '/', component: TaskList })
const boardRoute = createRoute({ getParentRoute: () => rootRoute, path: '/board', component: Board })
const reportsRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: '/reports',
  component: function ReportsRoute(): ReactElement {
    return (
      <Suspense fallback={<p>Loading…</p>}>
        <Reports />
      </Suspense>
    )
  },
})

const routeTree = rootRoute.addChildren([listRoute, boardRoute, reportsRoute])

export const router = createRouter({ routeTree })

declare module '@tanstack/react-router' {
  interface Register {
    router: typeof router
  }
}
