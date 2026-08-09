// Step 4 — step 3 plus @dnd-kit/core + @dnd-kit/sortable. This is the full
// docs/42 dependency set with no lazy route, so it is the honest floor.
import { StrictMode, type ReactElement } from 'react'
import { createRoot } from 'react-dom/client'
import { QueryClient, QueryClientProvider } from '@tanstack/react-query'
import {
  createRootRoute,
  createRoute,
  createRouter,
  Link,
  Outlet,
  RouterProvider,
} from '@tanstack/react-router'

import { Board } from './fixtures/Board'
import { TaskList } from './fixtures/TaskList'

function Shell(): ReactElement {
  return (
    <div>
      <Link to="/">Tasks</Link> <Link to="/board">Board</Link>
      <Outlet />
    </div>
  )
}

const rootRoute = createRootRoute({ component: Shell })
const routeTree = rootRoute.addChildren([
  createRoute({ getParentRoute: () => rootRoute, path: '/', component: TaskList }),
  createRoute({ getParentRoute: () => rootRoute, path: '/board', component: Board }),
])
const router = createRouter({ routeTree })

const queryClient = new QueryClient()
const host = document.getElementById('root')
if (host === null) throw new Error('#root missing')
createRoot(host).render(
  <StrictMode>
    <QueryClientProvider client={queryClient}>
      <RouterProvider router={router} />
    </QueryClientProvider>
  </StrictMode>,
)
