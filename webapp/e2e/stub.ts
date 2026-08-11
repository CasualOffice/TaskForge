/**
 * A server that answers in the API's own shapes, in the browser.
 *
 * Deliberately dumb: it exists so the app renders, not so an assertion passes.
 * Anything it does not know answers with an empty page, which is a real
 * response shape and not a special case.
 *
 * The rule these tests keep: **never assert on a value that came from here.**
 * A fixture written from the same assumption as the code confirms the
 * assumption — that is how a relations panel once shipped broken with a passing
 * test. What is asserted is geometry, which no fixture can fake.
 */
import type { Page } from '@playwright/test'

const WORKSPACE = '019fe000-0000-7000-8000-000000000001'
const PROJECT = '019fe000-0000-7000-8000-000000000002'
const TEAM = '019fe000-0000-7000-8000-000000000003'
const TASK = '019fe000-0000-7000-8000-000000000004'

const LONG_TITLE =
  'Fix the mobile task layout so the title survives a narrow screen instead of collapsing'

/** One task, in the wire's own shape. */
function task(): Record<string, unknown> {
  return {
    id: TASK,
    key: 'ONB-12',
    title: LONG_TITLE,
    type: 'BUG',
    priority: 'HIGH',
    state: 'ACTIVE',
    status_id: 's1',
    project_id: PROJECT,
    team_id: TEAM,
    environment_id: null,
    reporter_id: 'u1',
    due_at: null,
    start_at: null,
    description: null,
    parent_id: null,
    milestone_id: null,
    archived_at: null,
    version: 1,
    created_at: '2026-01-01T00:00:00Z',
    updated_at: '2026-01-02T00:00:00Z',
  }
}

function body(url: string): unknown {
  if (url.includes('/auth/session')) {
    return { user_id: 'u1', display_name: 'Test', email: 'test@example.test' }
  }
  if (url.includes('/api/v1/workspaces') && !url.includes('/teams')) {
    return { data: [{ id: WORKSPACE, name: 'Acme', slug: 'acme' }] }
  }
  if (/\/api\/v1\/me$/.test(url)) {
    return {
      id: 'u1',
      email: 'test@example.test',
      display_name: 'Test Person',
      avatar_url: null,
      time_zone: 'Europe/London',
    }
  }
  if (url.includes('/me/sessions')) {
    return {
      data: [
        {
          id: 's1',
          auth_method: 'PASSWORD',
          created_at: '2026-01-01T00:00:00Z',
          last_seen_at: '2026-01-02T00:00:00Z',
          expires_at: '2026-02-01T00:00:00Z',
          ip_address: null,
          user_agent: 'Firefox on a laptop',
          current: true,
        },
      ],
      page: { next_cursor: null, has_more: false },
    }
  }
  if (url.includes('/api/v1/roles')) {
    return {
      data: [
        { id: 'r1', name: 'Member', is_template: true, permissions: ['task.read'], version: 1 },
      ],
      page: { next_cursor: null, has_more: false },
    }
  }
  if (url.includes('/me/teams')) {
    return { data: [{ id: TEAM, name: 'Backend', created_at: '2026-01-01T00:00:00Z' }] }
  }
  if (url.includes('/api/v1/projects') && !url.includes('/tasks')) {
    return { data: [{ id: PROJECT, key: 'ONB', name: 'Onboarding', visibility: 'WORKSPACE' }] }
  }
  if (url.includes('/permissions/effective')) {
    return {
      workspace_id: WORKSPACE,
      actor_id: 'u1',
      project_id: null,
      permissions: [
        { permission: 'task.read', reach: 'unconditional' },
        { permission: 'task.create', reach: 'unconditional' },
        { permission: 'task.update', reach: 'unconditional' },
      ],
    }
  }
  // A report, in the shape every dashboard tile reads. The slices carry a long
  // name and a lopsided distribution on purpose: a chart only overflows its
  // tile when a label is too long for the column or a bar is at full width, and
  // a fixture of three tidy equal values would never produce either.
  if (url.includes('/api/v1/reports/run')) {
    return {
      group_by: 'assignee',
      measure: 'count',
      unit: 'tasks',
      total: 137,
      scope: { projects: 1 },
      groups: [
        { key: null, total: 94 },
        { key: 'u1', total: 31 },
        { key: 'u2', total: 8 },
        { key: 'u3', total: 4 },
      ],
    }
  }
  // One task, by id — the detail route reads this and the list reads the page
  // below it. Matched first, because `/tasks/{id}` also contains `/tasks`.
  if (/\/api\/v1\/tasks\/[0-9a-f-]+$/.test(url)) {
    return task()
  }
  if (url.includes('/api/v1/tasks?') || url.endsWith('/api/v1/tasks')) {
    return {
      data: [
        {
          ...task(),
          id: TASK,
          key: 'ONB-12',
          title: LONG_TITLE,
          type: 'BUG',
          priority: 'HIGH',
          state: 'ACTIVE',
          status_id: 's1',
          project_id: PROJECT,
          team_id: TEAM,
          environment_id: null,
          reporter_id: 'u1',
          due_at: null,
          start_at: null,
          description: null,
          parent_id: null,
          milestone_id: null,
          archived_at: null,
          version: 1,
          created_at: '2026-01-01T00:00:00Z',
          updated_at: '2026-01-02T00:00:00Z',
        },
      ],
      page: { next_cursor: null, has_more: false },
    }
  }
  return { data: [], page: { next_cursor: null, has_more: false } }
}

/** Answer every API call from the fixtures above. */
export async function stubApi(page: Page): Promise<void> {
  await page.route('**/api/v1/**', async (route) => {
    await route.fulfill({
      status: 200,
      contentType: 'application/json',
      body: JSON.stringify(body(route.request().url())),
    })
  })
  await page.addInitScript((id) => {
    window.localStorage.setItem('tf.workspace', id)
  }, WORKSPACE)
}
