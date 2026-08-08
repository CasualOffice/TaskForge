// Step 1 — step 0 plus @tanstack/react-query, exercised with one real query.
import { StrictMode, type ReactElement } from 'react'
import { createRoot } from 'react-dom/client'
import { QueryClient, QueryClientProvider, useQuery } from '@tanstack/react-query'

import { fetchTasks, type Task } from '../api'

function App(): ReactElement {
  const { data } = useQuery<Task[]>({ queryKey: ['tasks'], queryFn: () => fetchTasks(50) })
  return <p>{data?.length ?? 0}</p>
}

const queryClient = new QueryClient()
const host = document.getElementById('root')
if (host === null) throw new Error('#root missing')
createRoot(host).render(
  <StrictMode>
    <QueryClientProvider client={queryClient}>
      <App />
    </QueryClientProvider>
  </StrictMode>,
)
