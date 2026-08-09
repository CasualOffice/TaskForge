// Step 2 — step 1 plus @tanstack/react-router, with a two-route tree.
import { StrictMode, type ReactElement } from 'react'
import { createRoot } from 'react-dom/client'
import { QueryClient, QueryClientProvider, useQuery } from '@tanstack/react-query'
import {
  createRootRoute,
  createRoute,
  createRouter,
  Link,
  Outlet,
  RouterProvider,
} from '@tanstack/react-router'

import { fetchTasks, type Task } from './fixtures/tasks'

function Shell(): ReactElement {
  return (
    <div>
      <Link to="/">Tasks</Link> <Link to="/board">Board</Link>
      <Outlet />
    </div>
  )
}

function List(): ReactElement {
  const { data } = useQuery<Task[]>({ queryKey: ['tasks'], queryFn: () => fetchTasks(50) })
  return <p>{data?.length ?? 0}</p>
}

function Board(): ReactElement {
  return <p>board</p>
}

const rootRoute = createRootRoute({ component: Shell })
const routeTree = rootRoute.addChildren([
  createRoute({ getParentRoute: () => rootRoute, path: '/', component: List }),
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
