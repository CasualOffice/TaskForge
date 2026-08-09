/**
 * Does the app start?
 *
 * # The failure this file exists to prevent
 *
 * A tree that typechecks and does not boot. `tsc` proves the types agree; it
 * proves nothing about a provider mounted in the wrong order, an import cycle
 * between the router and a view, a hook called outside its context, or a module
 * that reads `localStorage` at import time. Every one of those is a blank white
 * page, and every one of them passes `pnpm typecheck && pnpm build`.
 *
 * The repository has already paid for this distinction once: `scripts/dev-up.sh`
 * exists because "the tests pass" and "you can open it" turned out to be very
 * different claims. This is the cheap half of that lesson — it runs in a second,
 * with no database and no browser.
 *
 * # Why the fetch is stubbed rather than mocked per endpoint
 *
 * The assertion is "it mounts and reaches a coherent state", not "it renders the
 * board correctly". One stub answering 401 puts the app in its signed-out state,
 * which is the state every visitor arrives in and the one the whole route tree
 * has to be mountable in.
 */
import { StrictMode } from 'react'
import { QueryClient, QueryClientProvider } from '@tanstack/react-query'
import { RouterProvider, createMemoryHistory, createRouter } from '@tanstack/react-router'
import { cleanup, render, screen, waitFor } from '@testing-library/react'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

import { routeTree } from './router'
import { Announcer } from './shell/announce'
import { SessionProvider } from './shell/session'

const unauthenticated = (): Response =>
  new Response(
    JSON.stringify({
      error: {
        code: 'TF-AUT-0001',
        message: 'unauthenticated',
        request_id: '018f2c00-0000-7000-8000-000000000000',
        docs: 'https://docs.taskforge.dev/errors/TF-AUT-0001',
      },
    }),
    { status: 401, headers: { 'content-type': 'application/json' } },
  )

beforeEach(() => {
  vi.stubGlobal('fetch', vi.fn(() => Promise.resolve(unauthenticated())))
  localStorage.clear()
})

afterEach(() => {
  cleanup()
  vi.unstubAllGlobals()
})

function mount(path: string) {
  const router = createRouter({
    routeTree,
    history: createMemoryHistory({ initialEntries: [path] }),
  })
  const client = new QueryClient({
    defaultOptions: { queries: { retry: false }, mutations: { retry: false } },
  })
  return render(
    <StrictMode>
      <QueryClientProvider client={client}>
        <SessionProvider>
          <Announcer>
            {/* eslint-disable-next-line @typescript-eslint/no-explicit-any -- a
                per-test router is not the registered singleton the module
                augmentation types `RouterProvider` against. */}
            <RouterProvider router={router as any} />
          </Announcer>
        </SessionProvider>
      </QueryClientProvider>
    </StrictMode>,
  )
}

describe('the app', () => {
  it('mounts and asks who the caller is', async () => {
    mount('/')
    await waitFor(() => {
      expect(fetch).toHaveBeenCalled()
    })
    const [url, init] = (fetch as unknown as { mock: { calls: [string, RequestInit][] } }).mock
      .calls[0] ?? ['', {}]
    expect(url).toContain('/api/v1/auth/session')
    // The session cookie is HttpOnly; without this it never travels and every
    // request is anonymous no matter how the user signed in.
    expect(init.credentials).toBe('include')
  })

  it('lands on the sign-in screen when nobody is signed in', async () => {
    mount('/board')
    // Not an error boundary, not a blank page: a 401 is the state every visitor
    // arrives in, and `readSession` folds it into a value for that reason.
    expect(await screen.findByLabelText('Email')).toBeDefined()
    expect(await screen.findByLabelText('Password')).toBeDefined()
  })

  it('mounts every route in the tree', async () => {
    // An import cycle or a hook-outside-context in one view fails only when that
    // route mounts, which is exactly the bug a single smoke test misses.
    for (const path of ['/', '/board', '/my-work', '/reports']) {
      const { unmount } = mount(path)
      expect(await screen.findByRole('heading', { name: 'TaskForge' })).toBeDefined()
      unmount()
    }
  })
})
