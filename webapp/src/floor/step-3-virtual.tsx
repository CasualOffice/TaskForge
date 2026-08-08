// Step 3 — step 2 plus @tanstack/react-virtual, via the shell's real list route.
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

import { TaskList } from '../routes/TaskList'

function Shell(): ReactElement {
  return (
    <div>
      <Link to="/">Tasks</Link> <Link to="/board">Board</Link>
      <Outlet />
    </div>
  )
}

function Board(): ReactElement {
  return <p>board</p>
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
