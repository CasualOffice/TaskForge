/**
 * The transport's edges.
 *
 * # Why an empty body has its own test
 *
 * It was a real bug, found by clicking the button rather than by any test here.
 * `POST /teams/{id}/members` answers `201` with no body; the transport
 * special-cased `204` and handed everything else to `response.json()`, which
 * threw a `SyntaxError` — not an `ApiError`, so it fell through to the generic
 * handler and the user was told "something went wrong on the server" about a
 * request the server had just succeeded at.
 *
 * Every write in the settings area goes through this function, so the failure
 * was one line away from every one of them. That is what makes it worth a test
 * rather than a comment.
 *
 * The fix for that bug then caused the opposite one: tolerating an empty body
 * everywhere meant a *read* that got nothing returned `undefined` under a type
 * promising a payload, and `/settings/workflow` crashed inside a child
 * component with "Cannot read properties of undefined (reading 'id')" — no
 * code, no request id, nothing pointing at the response. So the two cases are
 * now two functions, and both directions are held here.
 */
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

import { request, requestNoContent, requestWithVersion } from './http'
import { ApiError } from './problem'

function answering(init: {
  status: number
  body?: string
  headers?: Record<string, string>
}): void {
  vi.stubGlobal(
    'fetch',
    vi.fn(() =>
      Promise.resolve(
        new Response(init.body ?? null, {
          status: init.status,
          headers: init.headers ?? {},
        }),
      ),
    ),
  )
}

beforeEach(() => {
  // The CSRF token rides on an unsafe method; without it the header is simply
  // absent, which is what an unauthenticated tab looks like.
  document.cookie = 'tf_csrf=token'
})

afterEach(() => {
  vi.unstubAllGlobals()
})

describe('a response with no body', () => {
  it('resolves for a route that returns nothing, whatever the status', async () => {
    // `POST /teams/{id}/members` answers 201 with nothing, and adding someone
    // already in the team answers 200 with nothing. Both are successes.
    for (const status of [200, 201, 204]) {
      answering({ status })
      await expect(
        requestNoContent('/api/v1/teams/t1/members', { method: 'POST' }),
      ).resolves.toBeUndefined()
    }
  })

  it('is a fault when the caller was promised a payload', async () => {
    // The regression this file exists to hold: `request` used to hand back
    // `undefined` under a type that promised `T`. That sailed past every
    // `if (query.error)` and detonated later inside a child component as
    // "Cannot read properties of undefined", with no code and no request id.
    answering({ status: 200 })
    await expect(request<{ id: string }>('/api/v1/workflows/w1')).rejects.toThrow(ApiError)
  })

  it('is a fault for a versioned read, ETag or not', async () => {
    // Even with a usable version: `requestWithVersion`'s callers unwrap `.data`
    // into a child's props on the very next line.
    answering({ status: 200, headers: { etag: '"9"' } })
    await expect(requestWithVersion('/api/v1/workflows/w1')).rejects.toThrow(ApiError)
  })

  it('names the route it came from, so the fault is traceable', async () => {
    answering({ status: 200 })
    await expect(requestWithVersion('/api/v1/workflows/w1')).rejects.toMatchObject({
      code: 'TF-SYS-0001',
      message: expect.stringContaining('/api/v1/workflows/w1'),
    })
  })
})

describe('the version', () => {
  it('comes from the ETag even when the body has none', async () => {
    // `WorkspaceBody` carries no `version` field. A client reading only the body
    // could never send the `If-Match` its own rename requires.
    answering({
      status: 200,
      body: JSON.stringify({ id: 'w1', name: 'Acme', slug: 'acme' }),
      headers: { etag: '"4"', 'content-type': 'application/json' },
    })
    const { version } = await requestWithVersion('/api/v1/workspaces/w1')
    expect(version).toBe(4)
  })

  it('falls back to the body when no ETag arrives', async () => {
    answering({
      status: 200,
      body: JSON.stringify({ id: 't1', version: 7 }),
      headers: { 'content-type': 'application/json' },
    })
    const { version } = await requestWithVersion('/api/v1/tasks/t1')
    expect(version).toBe(7)
  })

  it('refuses a weak validator', async () => {
    // `W/"7"` means "equivalent, not identical" — precisely the guarantee
    // `If-Match` must not be given. Better to have no version and refuse to
    // write than to write conditionally on a promise nobody made.
    answering({ status: 200, body: JSON.stringify({ id: 't1' }), headers: { etag: 'W/"7"' } })
    const { version } = await requestWithVersion('/api/v1/tasks/t1')
    expect(version).toBeUndefined()
  })
})

describe('a refusal', () => {
  it('is an ApiError carrying the registry code', async () => {
    answering({
      status: 409,
      body: JSON.stringify({
        error: { code: 'TF-CNC-0001', message: 'stale', request_id: 'r1' },
      }),
      headers: { 'content-type': 'application/json' },
    })
    await expect(request('/api/v1/tasks/t1', { method: 'PATCH', ifMatch: 1 })).rejects.toMatchObject(
      { code: 'TF-CNC-0001', status: 409 },
    )
  })
})
