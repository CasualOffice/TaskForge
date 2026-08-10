/**
 * What the client actually asks the server for (`docs/45`, `docs/27`).
 *
 * # The gap this closes
 *
 * Four capabilities landed with their servers gated by Rust integration tests
 * and their clients gated by nothing: custody, releases, team scope, and the
 * type menu. `tsc` and `eslint` would not notice if picking a team stopped
 * narrowing the board — the code would still compile, still render, and quietly
 * show the whole workspace.
 *
 * # Why this asserts the outgoing request and not the rendered rows
 *
 * The rows come from a stub, so asserting them would assert the stub. What is
 * *not* the stub's is the URL the client builds: it is the contract with the
 * server, the server's half of it is already covered by the Rust suites, and
 * between the two the path is whole.
 *
 * This is also the mistake to avoid on purpose. A stub written from the same
 * assumption as the code confirms the assumption — that is exactly how a
 * relations panel once shipped broken with a passing test. So the fixtures here
 * answer with shapes taken from the API's own view types, and the assertions
 * are about the query string, which no fixture can fake.
 *
 * # What it still does not reach
 *
 * Layout, focus order, contrast, drag, and anything that needs pixels. Those
 * need a real browser, and `docs/15` still lists that row as missing.
 */
import { StrictMode, type ReactElement } from 'react'
import { QueryClient, QueryClientProvider } from '@tanstack/react-query'
import { RouterProvider, createMemoryHistory, createRouter } from '@tanstack/react-router'
import { render, cleanup, fireEvent, waitFor } from '@testing-library/react'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

import { routeTree } from './router'
import { Announcer } from './shell/announce'
import { SessionProvider } from './shell/session'

const WORKSPACE = '019fe000-0000-7000-8000-000000000001'
const PROJECT = '019fe000-0000-7000-8000-000000000002'
const TEAM = '019fe000-0000-7000-8000-000000000003'

/** Every request the render made, in order. */
let sent: string[] = []

/**
 * What `/permissions/effective` says about `task.create`.
 *
 * Absent `task_types` is the wire's way of saying "no narrowing", and it is a
 * different answer from an empty list — one of the two tests below is about
 * exactly that difference.
 */
let creatable: { task_types?: readonly string[] } = {}

/**
 * A server that answers in the API's own shapes.
 *
 * Deliberately dumb: it exists so the render completes, not so the assertions
 * pass. Anything it does not know answers with an empty page, which is a real
 * response shape and not a special case.
 */
function respond(url: string): unknown {
  if (url.startsWith('/api/v1/auth/session')) {
    return { user_id: 'u1', display_name: 'Test', email: 'test@example.test' }
  }
  if (url.startsWith('/api/v1/workspaces')) {
    return { data: [{ id: WORKSPACE, name: 'Acme', slug: 'acme' }] }
  }
  if (url.startsWith('/api/v1/projects')) {
    return {
      data: [{ id: PROJECT, key: 'ONB', name: 'Onboarding', visibility: 'WORKSPACE' }],
    }
  }
  if (url.startsWith('/api/v1/me/teams')) {
    return { data: [{ id: TEAM, name: 'Backend', created_at: '2026-01-01T00:00:00Z' }] }
  }
  if (url.startsWith('/api/v1/permissions/effective')) {
    return {
      workspace_id: WORKSPACE,
      actor_id: 'u1',
      project_id: null,
      permissions: [
        { permission: 'task.read', reach: 'unconditional' },
        { permission: 'task.create', reach: 'conditional', ...creatable },
      ],
    }
  }
  return { data: [], page: { next_cursor: null, has_more: false } }
}

beforeEach(() => {
  sent = []
  creatable = {}
  localStorage.setItem('tf.workspace', WORKSPACE)
  vi.stubGlobal(
    'fetch',
    vi.fn((input: RequestInfo | URL) => {
      const url = typeof input === 'string' ? input : input.toString()
      sent.push(url)
      return Promise.resolve(
        new Response(JSON.stringify(respond(url)), {
          status: 200,
          headers: { 'content-type': 'application/json' },
        }),
      )
    }),
  )
})

afterEach(() => {
  cleanup()
  vi.unstubAllGlobals()
})

function at(url: string): ReactElement {
  const router = createRouter({
    routeTree,
    history: createMemoryHistory({ initialEntries: [url] }),
  })
  const client = new QueryClient({
    defaultOptions: { queries: { retry: false, gcTime: 0 } },
  })
  return (
    <StrictMode>
      <QueryClientProvider client={client}>
        <SessionProvider>
          {/* The router's own type is the singleton's; this one is built per
              test so each starts at a different URL. */}
          <RouterProvider router={router as never} />
        </SessionProvider>
      </QueryClientProvider>
    </StrictMode>
  )
}

/** The task list request, once one has been made. */
async function taskQuery(): Promise<URLSearchParams> {
  const found = await waitFor(() => {
    const url = sent.find((u) => u.startsWith('/api/v1/tasks?'))
    expect(url, `no task list request was made; sent: ${sent.join(', ')}`).toBeDefined()
    return url as string
  })
  return new URLSearchParams(found.slice(found.indexOf('?') + 1))
}

describe('the scope in the address bar reaches the server', () => {
  it('carries a team scope into the task query', async () => {
    render(at(`/?team=${TEAM}`))
    expect((await taskQuery()).get('team')).toBe(TEAM)
  })

  it('carries the triage queue as a present-and-empty team', async () => {
    // `docs/27` §URL form: "`field=` — the empty value is how a URL says
    // 'unset'". Dropped, this reads as "every task in the workspace", which is
    // the dangerous direction: more rows than were asked for.
    render(at('/?team='))
    const query = await taskQuery()
    expect(query.has('team')).toBe(true)
    expect(query.get('team')).toBe('')
  })

  it('sends no team at all when none is scoped', async () => {
    // Distinct from the case above, and the distinction is the whole point.
    render(at(`/?project=${PROJECT}`))
    const query = await taskQuery()
    expect(query.has('team')).toBe(false)
    expect(query.get('project')).toBe(PROJECT)
  })

  it('carries both scopes together', async () => {
    render(at(`/?project=${PROJECT}&team=${TEAM}`))
    const query = await taskQuery()
    expect(query.get('project')).toBe(PROJECT)
    expect(query.get('team')).toBe(TEAM)
  })
})

describe('cutting a release', () => {
  it('posts the batch the reader selected, to the project they are in', async () => {
    // The client half of C-023. The server refuses a bad batch — that is
    // covered by `tests/releases.rs` — but nothing until now would notice if
    // the bar sent the wrong project, dropped a task, or sent the environment
    // under the wrong name, all of which produce a *successful* request for
    // the wrong thing.
    const { ReleaseBar } = await import('./views/ReleaseBar')
    const client = new QueryClient({ defaultOptions: { queries: { retry: false } } })
    const environments = [
      { id: 'env-staging', project_id: PROJECT, name: 'staging', position: 2 },
      { id: 'env-qa', project_id: PROJECT, name: 'qa', position: 1 },
    ]

    const view = render(
      <QueryClientProvider client={client}>
        <Announcer>
          <ReleaseBar
            workspaceId={WORKSPACE}
            projectId={PROJECT}
            environments={environments}
            selected={['task-1', 'task-2']}
            onCut={() => {}}
            onClear={() => {}}
          />
        </Announcer>
      </QueryClientProvider>,
    )

    const target = view.getByLabelText('Environment') as HTMLSelectElement
    fireEvent.change(target, { target: { value: 'env-staging' } })
    fireEvent.change(view.getByLabelText('Release name'), { target: { value: '2.4.0' } })
    fireEvent.click(view.getByRole('button', { name: 'Cut release' }))

    const posted = await waitFor(() => {
      const calls = (globalThis.fetch as ReturnType<typeof vi.fn>).mock.calls as unknown as [
        string,
        RequestInit,
      ][]
      const call = calls.find((entry) => String(entry[0]).includes('/releases'))
      expect(call, `no release was posted; sent: ${sent.join(', ')}`).toBeDefined()
      return call as [string, RequestInit]
    })

    expect(posted[0]).toBe(`/api/v1/projects/${PROJECT}/releases`)
    expect(posted[1].method).toBe('POST')
    expect(JSON.parse(String(posted[1].body))).toEqual({
      name: '2.4.0',
      environment_id: 'env-staging',
      task_ids: ['task-1', 'task-2'],
    })
  })

  it('offers nothing to press until it knows where the batch went', async () => {
    // A release with no environment records that something happened but not
    // where, which is not a release. Guarded in the control rather than
    // explained by a 400 afterwards.
    const { ReleaseBar } = await import('./views/ReleaseBar')
    const client = new QueryClient({ defaultOptions: { queries: { retry: false } } })
    const view = render(
      <QueryClientProvider client={client}>
        <Announcer>
          <ReleaseBar
            workspaceId={WORKSPACE}
            projectId={PROJECT}
            environments={[{ id: 'env-qa', project_id: PROJECT, name: 'qa', position: 1 }]}
            selected={['task-1']}
            onCut={() => {}}
            onClear={() => {}}
          />
        </Announcer>
      </QueryClientProvider>,
    )
    fireEvent.change(view.getByLabelText('Release name'), { target: { value: '2.4.0' } })
    const press = view.getByRole('button', { name: 'Cut release' }) as HTMLButtonElement
    expect(press.disabled).toBe(true)
  })
})

describe('the create form offers the types the actor may raise', () => {
  async function openTypeMenu(): Promise<HTMLSelectElement> {
    const { CreateTask } = await import('./views/CreateTask')
    const client = new QueryClient({ defaultOptions: { queries: { retry: false } } })
    const view = render(
      <QueryClientProvider client={client}>
        <SessionProvider>
          <Announcer>
            <CreateTask projectId={PROJECT} />
          </Announcer>
        </SessionProvider>
      </QueryClientProvider>,
    )
    fireEvent.click(await view.findByRole('button', { name: 'New task' }))
    fireEvent.click(await view.findByRole('button', { name: 'Type and priority' }))
    return view.getByLabelText('Type') as HTMLSelectElement
  }

  it('offers every type when the grant does not narrow by type', async () => {
    // `task_types` absent, which is not the same as empty.
    const select = await openTypeMenu()
    await waitFor(() => expect(select.options.length).toBeGreaterThan(2))
    expect([...select.options].map((o) => o.value)).toContain('FEATURE')
  })

  it('offers only the named types, and defaults to one of them', async () => {
    // The client half of C-025. The server refuses a type outside the grant,
    // but a form that offers `TASK` to someone who may only raise `BUG` sends
    // them into a 403 for pressing the default — which is the cognitive
    // burden this product exists to remove.
    creatable = { task_types: ['BUG', 'INCIDENT'] }
    const select = await openTypeMenu()
    await waitFor(() => {
      expect([...select.options].map((o) => o.value)).toEqual(['BUG', 'INCIDENT'])
    })
    // And the default moved off `TASK`, which nobody here may raise.
    expect(select.value).toBe('BUG')
  })
})
