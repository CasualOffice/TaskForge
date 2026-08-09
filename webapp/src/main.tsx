/**
 * Mount.
 *
 * # Why the theme is applied before React renders
 *
 * `apply()` sets `data-theme` on the document element synchronously, so the page
 * is painted in the user's theme rather than repainted into it. A dark-theme
 * user who sees a white flash on every load is being told, once per visit, that
 * the app is not quite finished.
 */
import { StrictMode } from 'react'
import { createRoot } from 'react-dom/client'
import { QueryClient, QueryClientProvider } from '@tanstack/react-query'
import { RouterProvider } from '@tanstack/react-router'

import { router } from './router'
import { Announcer } from './shell/announce'
import { SessionProvider } from './shell/session'
import { apply, storedChoice } from './shell/theme'
import './styles/app.css'
import './styles/signin.css'
import './styles/my-work.css'
import './styles/authenticated-shell.css'
import './styles/workspace-surfaces.css'
import './styles/settings-premium.css'

const queryClient = new QueryClient({
  defaultOptions: {
    queries: {
      // A refusal is an answer, not a flake. Retrying a 403 four times spends
      // four round trips learning the same thing and delays the message the
      // user needs by a second and a half.
      retry: (failureCount, error) => {
        const status = (error as { status?: number }).status ?? 0
        if (status >= 400 && status < 500) return false
        return failureCount < 2
      },
      refetchOnWindowFocus: false,
    },
  },
})

apply(storedChoice())

const host = document.getElementById('root')
if (host === null) throw new Error('#root missing')

createRoot(host).render(
  <StrictMode>
    <QueryClientProvider client={queryClient}>
      <SessionProvider>
        <Announcer>
          <RouterProvider router={router} />
        </Announcer>
      </SessionProvider>
    </QueryClientProvider>
  </StrictMode>,
)
