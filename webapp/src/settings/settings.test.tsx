/**
 * Every settings screen, mounted against a stubbed API.
 *
 * # Why this is not covered by `boot.test.tsx`
 *
 * That file mounts the route tree signed *out*, so `AppFrame` renders the
 * sign-in screen and no settings component is ever constructed. Everything these
 * screens get wrong — a hook outside its provider, a lazy chunk that fails to
 * resolve, a list that renders `undefined` because the envelope was `{data}` and
 * the code expected an array — happens only after a real payload arrives.
 *
 * # Why the stub answers by URL
 *
 * Each screen makes three to five calls, and a single canned response would
 * make every assertion a test of the stub. Routing by path means the payloads
 * are the *server's* shapes — copied from the wire types — so a screen reading a
 * field the API does not send fails here rather than in a browser.
 *
 * # What is asserted
 *
 * That the screen renders what it was given, that the controls a permission
 * gates are present when it is held, and that axe finds nothing. Not the
 * writes: those go through `useWrite`, and asserting a mutation fired proves the
 * click handler is wired, not that the server accepts it — which is what the
 * Rust integration tests are for.
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

/**
 * The default is one second, and the chain here needs a little more: session,
 * then the workspace list, then the effective permissions, then the screen's own
 * queries — each a hop, each rendered twice under `StrictMode`. Measured at
 * ~800 ms, so this is headroom rather than a guess, and it stays well under
 * vitest's own 5 s per-test deadline so a genuine hang still fails as a hang.
 */
configure({ asyncUtilTimeout: 2_000 })

const WORKSPACE = '019200000000700080000000000000w1'
const PROJECT = '019200000000700080000000000000p1'
const WORKFLOW = '019200000000700080000000000000f1'
const ME = '019200000000700080000000000000u1'

/** Every permission, so no screen is gated in these tests. */
const EVERYTHING = [
  'workspace.manage',
  'role.manage',
  'role.assign',
  'tag.manage',
  'project.workflow.manage',
].map((permission) => ({ permission, reach: 'unconditional' }))

const STATUSES = [
  { id: 's1', name: 'Backlog', state: 'BACKLOG', position: 1, is_initial: true },
  { id: 's2', name: 'Todo', state: 'PLANNED', position: 2, is_initial: false },
  { id: 's3', name: 'Done', state: 'COMPLETED', position: 3, is_initial: false },
]

/**
 * The canned API. Longest match wins, so `/me/sessions` is not answered by the
 * `/me` entry above it.
 */
const RESPONSES: ReadonlyArray<readonly [string, unknown]> = [
  ['/api/v1/auth/session', { actor_id: ME, actor_type: 'USER' }],
  [
    '/api/v1/workspaces?',
    { data: [{ id: WORKSPACE, name: 'Acme', slug: 'acme', created_at: iso() }], page: { has_more: false } },
  ],
  [`/api/v1/workspaces/${WORKSPACE}/members`, { data: [member()], page: { has_more: false } }],
  [`/api/v1/workspaces/${WORKSPACE}/invitations`, { data: [invitation()], page: { has_more: false } }],
  [
    `/api/v1/workspaces/${WORKSPACE}/teams`,
    { data: [{ id: 't1', name: 'Platform', created_at: iso() }], page: { has_more: false } },
  ],
  [`/api/v1/workspaces/${WORKSPACE}`, { id: WORKSPACE, name: 'Acme', slug: 'acme', created_at: iso() }],
  ['/api/v1/teams/t1/members', { data: [member()], page: { has_more: false } }],
  ['/api/v1/me/sessions', { data: [session()] }],
  ['/api/v1/me', me()],
  ['/api/v1/permissions/effective', { workspace_id: WORKSPACE, actor_id: ME, project_id: null, permissions: EVERYTHING }],
  ['/api/v1/roles', { data: [role()] }],
  ['/api/v1/role-assignments', { data: [assignment()], page: { has_more: false } }],
  ['/api/v1/projects', { data: [project()], page: { has_more: false } }],
  [`/api/v1/workflows/${WORKFLOW}/statuses`, { data: STATUSES.map((s) => ({ ...s, task_count: 3 })) }],
  [`/api/v1/workflows/${WORKFLOW}`, workflow()],
  ['/api/v1/tags', { data: [{ id: 'g1', project_id: null, name: 'security', color: '#7a5cff' }] }],
]

function iso(): string {
  return '2026-08-09T10:00:00Z'
}
function me(): unknown {
  return { id: ME, email: 'dev@example.test', display_name: 'Ada', avatar_url: null, time_zone: 'Europe/London' }
}
function member(): unknown {
  return { user_id: ME, display_name: 'Ada', email: 'dev@example.test', member_type: 'MEMBER', joined_at: iso() }
}
function session(): unknown {
  return {
    id: 'x1',
    auth_method: 'PASSWORD',
    created_at: iso(),
    last_seen_at: iso(),
    expires_at: iso(),
    ip_address: '203.0.113.7',
    user_agent: 'Firefox',
    current: true,
  }
}
function invitation(): unknown {
  return { id: 'i1', email: 'new@example.test', role_id: 'r1', invited_by: ME, expires_at: iso(), created_at: iso() }
}
function role(): unknown {
  return {
    id: 'r1',
    name: 'Reader',
    is_template: false,
    permissions: ['task.read'],
    created_at: iso(),
    updated_at: iso(),
    version: 1,
  }
}
function assignment(): unknown {
  return {
    id: 'a1',
    principal_type: 'USER',
    principal_id: ME,
    role_id: 'r1',
    scope_type: 'WORKSPACE',
    scope_id: WORKSPACE,
    granted_by: ME,
    granted_at: iso(),
  }
}
function project(): unknown {
  return {
    id: PROJECT,
    key: 'WR',
    name: 'Work',
    description: null,
    visibility: 'WORKSPACE',
    team_id: null,
    workflow_id: WORKFLOW,
    created_at: iso(),
    created_by: ME,
    updated_at: iso(),
    updated_by: null,
    archived_at: null,
    version: 1,
  }
}
function workflow(): unknown {
  return {
    id: WORKFLOW,
    name: 'Default',
    is_default: true,
    version: 4,
    statuses: STATUSES,
    // Backlog → Todo only. The matrix test reads both the present cell and an
    // absent one, so a component that drew every cell the same would fail.
    transitions: [
      { id: 'e1', from: 's1', to: 's2', required_permission: null, required_fields: [], ignore_dependencies: false },
    ],
  }
}

function answer(url: string): Response {
  const path = url.replace(/^https?:\/\/[^/]+/, '')
  const match = [...RESPONSES]
    .filter(([prefix]) => path.startsWith(prefix))
    .sort((a, b) => b[0].length - a[0].length)[0]
  if (match === undefined) {
    return new Response(JSON.stringify({ error: { code: 'TF-AZN-0008', message: 'not stubbed', request_id: 'r' } }), {
      status: 404,
      headers: { 'content-type': 'application/json' },
    })
  }
  return new Response(JSON.stringify(match[1]), {
    status: 200,
    // The version the settings writes need. `WorkspaceBody` carries none in its
    // body, so a screen that read only the body would send no `If-Match`.
    headers: { 'content-type': 'application/json', etag: '"4"' },
  })
}

beforeEach(() => {
  vi.stubGlobal('fetch', vi.fn((input: RequestInfo | URL) => Promise.resolve(answer(String(input)))))
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

async function clean(container: Element): Promise<string> {
  const results = await axe.run(container, { rules: { 'color-contrast': { enabled: false } } })
  return results.violations.map((v) => `${v.id}: ${v.help} (${v.nodes.length})`).join('\n')
}

describe('the settings shell', () => {
  it('offers every section as a link', async () => {
    render(mount('/settings/profile'))
    const nav = await screen.findByRole('navigation', { name: 'Settings' })
    for (const label of ['Your profile', 'Workspace', 'Members', 'Teams', 'Roles', 'Workflow', 'Tags']) {
      expect(nav.textContent).toContain(label)
    }
  })
})

describe('the profile screen', () => {
  it('renders the account, the zone, and where you are signed in', async () => {
    const { container } = render(mount('/settings/profile'))
    expect(await screen.findByDisplayValue('Ada')).toBeDefined()
    expect(await screen.findByDisplayValue('Europe/London')).toBeDefined()
    // The current session is listed and offers no "sign out" for itself: doing
    // so would sign the reader out of the page they are standing on.
    const row = (await screen.findByText(/Firefox/)).closest('li')
    expect(row).not.toBeNull()
    // No sign-out control on the current session: it would sign the reader out
    // of the page they are standing on. The shell's own Sign out is elsewhere,
    // which is why this is scoped to the row rather than to the document.
    expect(row?.querySelector('button')).toBeNull()
    expect(row?.textContent).toContain('this session')
    expect(await clean(container)).toBe('')
  })
})

describe('the members screen', () => {
  it('says when a member holds no role, and lists invitations', async () => {
    const { container } = render(mount('/settings/members'))
    const row = (await screen.findByText(/dev@example.test/)).closest('li')
    expect(row).not.toBeNull()
    // Ada holds Reader in the stub, so her row names it and does *not* carry the
    // "no roles" sentence — which is the assertion that the grant list was
    // actually joined onto the member list rather than rendered beside it.
    await waitFor(() => expect(row?.textContent).toContain('Reader'))
    expect(row?.textContent).not.toContain('no roles')
    expect(await screen.findByText('new@example.test')).toBeDefined()
    expect(await clean(container)).toBe('')
  })
})

describe('the roles screen', () => {
  it('lists roles and every permission the server knows', async () => {
    const { container } = render(mount('/settings/roles'))
    // Several: the role list, the grant list, and the role picker's option.
    expect((await screen.findAllByText(/Reader/)).length).toBeGreaterThan(0)
    const edit = await screen.findByRole('button', { name: 'Edit' })
    edit.click()
    // One checkbox per permission, each labelled by its key — a picker whose
    // boxes had no accessible name would pass a render test and fail a person.
    expect(await screen.findByLabelText(/task\.transition/)).toBeDefined()
    expect(await screen.findByLabelText(/role\.manage/)).toBeDefined()
    expect(await clean(container)).toBe('')
  })

  it('shows who holds what, with a name rather than an id', async () => {
    render(mount('/settings/roles'))
    await waitFor(() => expect(screen.getByText(/Ada — Reader/)).toBeDefined())
    expect(screen.queryByText(new RegExp(ME))).toBeNull()
  })
})

describe('the workflow screen', () => {
  it('draws the transition matrix with allowed and disallowed cells', async () => {
    const { container } = render(mount('/settings/workflow'))
    // The stub allows Backlog → Todo and nothing else.
    expect(await screen.findByRole('button', { name: 'Backlog to Todo: allowed' })).toBeDefined()
    expect(await screen.findByRole('button', { name: 'Backlog to Done: not allowed' })).toBeDefined()
    expect(await clean(container)).toBe('')
  })

  it('shows each status’s permanent state and how many tasks sit on it', async () => {
    render(mount('/settings/workflow'))
    // The state is not inferable from the name, and getting it wrong makes every
    // report wrong quietly. It is on every row for that reason.
    await waitFor(() => expect(screen.getByText(/completed · 3 tasks/)).toBeDefined())
  })
})

describe('the teams and tags screens', () => {
  it('render their lists with no axe violations', async () => {
    const teams = render(mount('/settings/teams'))
    expect(await screen.findByText('Platform')).toBeDefined()
    expect(await clean(teams.container)).toBe('')
    cleanup()

    const tags = render(mount('/settings/tags'))
    expect(await screen.findByText('security')).toBeDefined()
    expect(await clean(tags.container)).toBe('')
  })
})

describe('the workspace screen', () => {
  it('shows the slug as permanent and the name as editable', async () => {
    const { container } = render(mount('/settings/workspace'))
    const name = (await screen.findByLabelText('Name')) as HTMLInputElement
    const slug = (await screen.findByLabelText('Identifier')) as HTMLInputElement
    expect(name.value).toBe('Acme')
    expect(slug.readOnly).toBe(true)
    expect(await clean(container)).toBe('')
  })
})
