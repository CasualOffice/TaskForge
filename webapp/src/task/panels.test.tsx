/**
 * The task surface's panels, against a stubbed API.
 *
 * # What is actually at risk here
 *
 * Not layout — the assertions below are about *meaning*. Three of these panels
 * render facts that are wrong in a way a render test would not notice:
 *
 * - a blocker the viewer cannot see must appear as `restricted`, not vanish;
 *   dropping the row shows a task as blocked by nothing, which reads as "you may
 *   move this" and is the opposite of true (`docs/03`);
 * - subtask progress must come from the server's counts, not from the page
 *   rendered, or a task with forty children reports "3 of 5 done";
 * - an activity row for an event type the client has never seen must still be a
 *   row, because a dropped row is a hole in an audit trail.
 *
 * # And one that is about the request, not the render
 *
 * Activity is not fetched until the disclosure is opened. On a board where the
 * peek opens on hover, fetching eagerly is one history request per card looked
 * at, and nothing on screen would show it happening.
 */
import { StrictMode, type ReactElement } from 'react'
import { QueryClient, QueryClientProvider } from '@tanstack/react-query'
import { RouterProvider, createMemoryHistory, createRouter } from '@tanstack/react-router'
import { cleanup, configure, render, screen, waitFor } from '@testing-library/react'
import axe from 'axe-core'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

import { routeTree } from '../router'
import { Announcer } from '../shell/announce'
import { SessionProvider } from '../shell/session'

configure({ asyncUtilTimeout: 2_000 })

const WORKSPACE = '019200000000700080000000000000w1'
const PROJECT = '019200000000700080000000000000p1'
const TASK = '019200000000700080000000000000t1'
const CHILD = '019200000000700080000000000000t2'
const BLOCKER = '019200000000700080000000000000t3'
const ME = '019200000000700080000000000000u1'

const PERMISSIONS = ['task.read', 'task.update', 'task.assign', 'task.history.read'].map(
  (permission) => ({ permission, reach: 'unconditional' }),
)

function iso(): string {
  return '2026-08-09T10:00:00Z'
}

function task(id: string, key: string, title: string, state = 'ACTIVE'): unknown {
  return {
    id,
    key,
    project_id: PROJECT,
    number: 1,
    title,
    description: null,
    type: 'TASK',
    priority: 'MEDIUM',
    status_id: 's1',
    state,
    reporter_id: ME,
    environment_id: null,
    milestone_id: null,
    parent_id: null,
    start_at: null,
    due_at: null,
    position: 'n',
    created_at: iso(),
    created_by: ME,
    updated_at: iso(),
    updated_by: null,
    archived_at: null,
    is_blocked: true,
    version: 1,
  }
}

const RESPONSES: ReadonlyArray<readonly [string, unknown]> = [
  ['/api/v1/auth/session', { actor_id: ME, actor_type: 'USER' }],
  [
    '/api/v1/workspaces?',
    {
      data: [{ id: WORKSPACE, name: 'Acme', slug: 'acme', created_at: iso() }],
      page: { has_more: false },
    },
  ],
  [
    `/api/v1/workspaces/${WORKSPACE}/members`,
    {
      data: [
        {
          user_id: ME,
          display_name: 'Ada',
          email: 'dev@example.test',
          member_type: 'MEMBER',
          joined_at: iso(),
        },
      ],
      page: { has_more: false },
    },
  ],
  [
    '/api/v1/permissions/effective',
    { workspace_id: WORKSPACE, actor_id: ME, project_id: null, permissions: PERMISSIONS },
  ],
  [
    '/api/v1/projects',
    {
      data: [
        {
          id: PROJECT,
          key: 'WR',
          name: 'Work',
          description: null,
          visibility: 'WORKSPACE',
          team_id: null,
          workflow_id: 'f1',
          created_at: iso(),
          created_by: ME,
          updated_at: iso(),
          updated_by: null,
          archived_at: null,
          version: 1,
        },
      ],
      page: { has_more: false },
    },
  ],
  [`/api/v1/tasks/${TASK}/assignees`, { assignees: [ME] }],
  [
    `/api/v1/tasks/${TASK}/dependencies`,
    {
      blocked_by: [
        { id: BLOCKER, key: 'WR-9', title: 'Ship the API', state: 'ACTIVE', restricted: false },
        // The one that matters: an edge whose far end is invisible.
        { id: null, key: null, title: null, state: null, restricted: true },
      ],
      blocks: [
        { id: CHILD, key: 'WR-3', title: 'Already done', state: 'COMPLETED', restricted: false },
      ],
    },
  ],
  [
    `/api/v1/tasks/${TASK}/subtasks`,
    {
      parent: { id: BLOCKER, key: 'WR-9', title: 'Ship the API', state: 'ACTIVE' },
      // Flat, not nested: `Relationship` flattens `Subtasks` into itself. The
      // first version of this stub nested them under `children`, matched the
      // client's wrong type, and passed while the real surface crashed.
      data: [task(CHILD, 'WR-3', 'A child')],
      done: 7,
      total: 40,
      truncated: true,
    },
  ],
  [
    `/api/v1/tasks/${TASK}/activity`,
    {
      data: [
        {
          id: 'a1',
          event_type: 'task.status.changed',
          actor_id: ME,
          actor_name: 'Ada',
          changes: { status: { from: 'Todo', to: 'In Progress' } },
          occurred_at: iso(),
        },
        {
          id: 'a2',
          event_type: 'task.something.nobody.wrote.yet',
          actor_id: null,
          actor_name: null,
          changes: {},
          occurred_at: iso(),
        },
      ],
      page: { has_more: false },
    },
  ],
  [`/api/v1/tasks/${TASK}/comments`, { data: [], page: { has_more: false } }],
  [
    `/api/v1/tasks/${TASK}/custody`,
    { team_id: null, environment_id: null, transfers: [], promotions: [], verifications: [] },
  ],
  [`/api/v1/projects/${PROJECT}/environments`, { data: [] }],
  [
    `/api/v1/workspaces/${WORKSPACE}/teams`,
    { data: [{ id: 'tm1', name: 'Android', created_at: iso() }], page: { has_more: false } },
  ],
  [`/api/v1/tasks/${TASK}`, task(TASK, 'WR-1', 'The task under test')],
]

function answer(url: string): Response {
  const path = url.replace(/^https?:\/\/[^/]+/, '')
  const match = [...RESPONSES]
    .filter(([prefix]) => path.startsWith(prefix))
    .sort((a, b) => b[0].length - a[0].length)[0]
  if (match === undefined) {
    return new Response(
      JSON.stringify({ error: { code: 'TF-AZN-0008', message: 'not stubbed', request_id: 'r' } }),
      { status: 404, headers: { 'content-type': 'application/json' } },
    )
  }
  return new Response(JSON.stringify(match[1]), {
    status: 200,
    headers: { 'content-type': 'application/json', etag: '"1"' },
  })
}

function calls(): string[] {
  return (fetch as unknown as { mock: { calls: [string][] } }).mock.calls.map((c) => String(c[0]))
}

beforeEach(() => {
  vi.stubGlobal(
    'fetch',
    vi.fn((input: RequestInfo | URL) => Promise.resolve(answer(String(input)))),
  )
  localStorage.clear()
})

afterEach(() => {
  cleanup()
  vi.unstubAllGlobals()
})

function mount(path: string): ReactElement {
  const router = createRouter({ routeTree, history: createMemoryHistory({ initialEntries: [path] }) })
  const client = new QueryClient({
    defaultOptions: { queries: { retry: false }, mutations: { retry: false } },
  })
  return (
    <StrictMode>
      <QueryClientProvider client={client}>
        <SessionProvider>
          <Announcer>
            {/* eslint-disable-next-line @typescript-eslint/no-explicit-any -- a
                per-test router is not the registered singleton. */}
            <RouterProvider router={router as any} />
          </Announcer>
        </SessionProvider>
      </QueryClientProvider>
    </StrictMode>
  )
}

describe('the blockers panel', () => {
  it('draws an invisible blocker as restricted rather than dropping it', async () => {
    render(mount(`/tasks/${TASK}`))
    expect((await screen.findAllByText(/Ship the API/)).length).toBeGreaterThan(0)
    // Dropping this row would show the task as blocked by nothing.
    expect(await screen.findByText('A task in a project you cannot see')).toBeDefined()
  })

  it('keeps blocked-by and blocks as two lists', async () => {
    // §12: a blocker gates this task's transitions; a task this one blocks does
    // not. One merged list puts a thing that stops you beside one that does not.
    render(mount(`/tasks/${TASK}`))
    expect(await screen.findByRole('heading', { name: 'Blocked by' })).toBeDefined()
    expect(await screen.findByRole('heading', { name: 'Blocks' })).toBeDefined()
  })

  it('offers removal per row, naming the task in the label', async () => {
    render(mount(`/tasks/${TASK}`))
    expect(
      await screen.findByRole('button', { name: 'Remove the dependency on WR-9' }),
    ).toBeDefined()
    // One per identifiable row — the blocker and the blocked task — and none on
    // the restricted row, which has no id to name in the call.
    const buttons = await screen.findAllByRole('button', { name: /Remove the dependency/ })
    expect(buttons.map((b) => b.getAttribute('aria-label'))).toEqual([
      'Remove the dependency on WR-9',
      'Remove the dependency on WR-3',
    ])
  })
})

describe('the subtasks panel', () => {
  it('reports progress from the server’s counts, not from the page', async () => {
    // One child is rendered; the server says 7 of 40 are done. A panel counting
    // what it drew would say "0 of 1".
    render(mount(`/tasks/${TASK}`))
    expect(await screen.findByText('7 of 40 done')).toBeDefined()
    expect(await screen.findByText(/Showing the first 1 of 40/)).toBeDefined()
  })

  it('links to the parent', async () => {
    render(mount(`/tasks/${TASK}`))
    expect(await screen.findByRole('heading', { name: 'Part of' })).toBeDefined()
  })
})

describe('the activity panel', () => {
  it('is not fetched until it is opened', async () => {
    render(mount(`/tasks/${TASK}`))
    await screen.findAllByText(/Ship the API/)
    expect(calls().filter((url) => url.includes('/activity'))).toHaveLength(0)

    const toggle = await screen.findByRole('button', { name: 'Activity' })
    toggle.click()
    await waitFor(() =>
      expect(calls().filter((url) => url.includes('/activity')).length).toBeGreaterThan(0),
    )
  })

  it('renders a known event as a sentence and an unknown one as itself', async () => {
    render(mount(`/tasks/${TASK}`))
    const toggle = await screen.findByRole('button', { name: 'Activity' })
    toggle.click()
    expect(await screen.findByText('moved it from Todo to In Progress')).toBeDefined()
    // A dropped row is a hole in an audit trail. Ugly and true beats absent.
    expect(await screen.findByText('task.something.nobody.wrote.yet')).toBeDefined()
    // A null actor is the system, not "someone". Scoped to the row, because the
    // shell's wordmark says TaskForge too.
    const row = (await screen.findByText('task.something.nobody.wrote.yet')).closest('li')
    expect(row?.textContent).toContain('TaskForge')
  })
})

describe('the assignee field', () => {
  it('shows who is assigned, read from its own endpoint', async () => {
    render(mount(`/tasks/${TASK}`))
    // "Not shown yet" was the old answer, when the set was write-only.
    await waitFor(() => expect(screen.getAllByText(/Ada/).length).toBeGreaterThan(0))
    expect(screen.queryByText('Not shown yet')).toBeNull()
  })
})

describe('the whole surface', () => {
  it('has no axe violations with every panel rendered', async () => {
    const { container } = render(mount(`/tasks/${TASK}`))
    await screen.findAllByText(/Ship the API/)
    const results = await axe.run(container, { rules: { 'color-contrast': { enabled: false } } })
    expect(results.violations.map((v) => `${v.id}: ${v.help}`).join('\n')).toBe('')
  })
})
