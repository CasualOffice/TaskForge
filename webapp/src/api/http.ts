/**
 * The transport. One place that knows how a TaskForge request is shaped.
 *
 * # The failure this module prevents
 *
 * A mutation that reaches the server without its CSRF token, its workspace, or
 * its `If-Match` — and therefore fails at the edge with a refusal the user reads
 * as "the app is broken". `docs/05` §Authentication and `docs/40` put four
 * obligations on every browser call:
 *
 * - the session cookie rides along (`credentials: 'include'`),
 * - unsafe methods carry `x-csrf-token`, read from the non-HttpOnly `tf_csrf`
 *   cookie the login response set,
 * - workspace-scoped reads carry `x-workspace-id`,
 * - conditional writes carry `If-Match` and creates carry `Idempotency-Key`.
 *
 * Every one of those is easy to forget at a call site and impossible to forget
 * here, which is why no route component is allowed to call `fetch` directly.
 */
import { ApiError, type ErrorEnvelope } from './problem'

/**
 * Same-origin, always, unless a deployment overrides it.
 *
 * The session cookie is `HttpOnly` and `SameSite=Lax` and the API registers no
 * CORS layer, so a cross-origin base would fail on every authenticated request.
 * `pnpm dev` proxies `/api` to the API process for exactly that reason
 * (`vite.config.ts`), which also means development and production have the same
 * origin shape rather than two different ones.
 */
const BASE = import.meta.env['VITE_API_BASE'] ?? ''

/** `docs/05` §Authentication. Validated server-side against membership. */
const WORKSPACE_HEADER = 'x-workspace-id'
/** `crates/casual-task-api/src/csrf.rs` — the header half of the double submit. */
const CSRF_HEADER = 'x-csrf-token'
/** The cookie half. Deliberately not `HttpOnly`: the client has to echo it. */
const CSRF_COOKIE = 'tf_csrf'

/** Methods that change state, and therefore need a CSRF token. */
const UNSAFE = new Set(['POST', 'PUT', 'PATCH', 'DELETE'])

export interface RequestOptions {
  readonly method?: string
  readonly body?: unknown
  /** The workspace this call is scoped to. Omitted only by the auth routes. */
  readonly workspaceId?: string | undefined
  /** The `version` from a previous read. Required by every conditional write. */
  readonly ifMatch?: number | undefined
  /** `docs/24`: required by `POST /projects/{id}/tasks`. */
  readonly idempotencyKey?: string | undefined
  readonly signal?: AbortSignal | undefined
}

/**
 * The CSRF token, from the cookie the server set.
 *
 * Read per request rather than cached at login: a session that outlives the tab
 * (the ordinary case — the cookie lasts 30 days) has a valid token no login
 * response ever handed this process.
 */
export function csrfToken(): string | undefined {
  for (const pair of document.cookie.split(';')) {
    const [name, ...rest] = pair.trim().split('=')
    if (name === CSRF_COOKIE) return rest.join('=')
  }
  return undefined
}

/**
 * Perform a request and return its parsed body.
 *
 * @throws {ApiError} for any non-2xx response, carrying the registry code from
 * `docs/20` so the caller can decide what the user can do about it. A raw body
 * is never surfaced — `ApiError.fromResponse` normalizes even a proxy's HTML.
 */
export async function request<T>(path: string, options: RequestOptions = {}): Promise<T> {
  return (await send<T>(path, options)).data
}

/**
 * The request, with the response's `ETag` beside its body.
 *
 * Split out of [`request`] rather than layered over it because the version only
 * exists on the `Response`, and by the time `request` has parsed the body it has
 * thrown the headers away.
 */
async function send<T>(
  path: string,
  options: RequestOptions,
): Promise<{ data: T; etag: string | null }> {
  const method = options.method ?? 'GET'
  const headers = new Headers({ accept: 'application/json' })

  if (options.body !== undefined) headers.set('content-type', 'application/json')
  if (options.workspaceId !== undefined) headers.set(WORKSPACE_HEADER, options.workspaceId)
  if (options.ifMatch !== undefined) headers.set('if-match', `"${options.ifMatch}"`)
  if (options.idempotencyKey !== undefined) {
    headers.set('idempotency-key', options.idempotencyKey)
  }
  if (UNSAFE.has(method)) {
    const token = csrfToken()
    if (token !== undefined) headers.set(CSRF_HEADER, token)
  }

  let response: Response
  try {
    response = await fetch(`${BASE}${path}`, {
      method,
      headers,
      // The session cookie is HttpOnly, so this is the only way it travels.
      credentials: 'include',
      body: options.body === undefined ? undefined : JSON.stringify(options.body),
      ...(options.signal === undefined ? {} : { signal: options.signal }),
    })
  } catch (cause) {
    // A DNS failure, an offline tab, or a CORS refusal. Given its own code
    // rather than a generic throw so the UI can say "you appear to be offline"
    // instead of "TF-SYS-0001", which would blame a server that never answered.
    throw ApiError.offline(cause)
  }

  if (!response.ok) throw await ApiError.fromResponse(response)
  const etag = response.headers.get('etag')

  // An empty body is not only a 204. `POST /teams/{id}/members` answers 201 with
  // nothing in it, and adding someone already in the team answers 200 with
  // nothing in it — both perfectly reasonable, and both fed straight into
  // `response.json()` by the version of this function that special-cased 204
  // alone. That throws a `SyntaxError`, which is not an `ApiError`, so it
  // reached the user as "something went wrong on the server" for a request the
  // server had just succeeded at. Reading the text first is the fix: whether a
  // body is present is a fact about the response, not a fact about its status.
  const text = await response.text()
  if (text === '') return { data: undefined as T, etag }
  return { data: JSON.parse(text) as T, etag }
}

/**
 * A request whose `ETag` the caller needs.
 *
 * # The header, then the body — and both are load-bearing
 *
 * `docs/05` echoes `version` in most bodies, and that is convenient, but it is
 * not universal: `WorkspaceBody` carries `id`, `name`, `slug` and `created_at`
 * and no version at all, so a settings page that read only the body could never
 * send the `If-Match` its own rename requires. The `ETag` is the contract —
 * every conditional resource sets it — and the body is the fallback for the day
 * an intermediary strips it.
 */
export async function requestWithVersion<T>(
  path: string,
  options: RequestOptions = {},
): Promise<{ data: T; version: number | undefined }> {
  const { data, etag } = await send<T>(path, options)
  return { data, version: parseEtag(etag) ?? (data as { version?: number } | undefined)?.version }
}

/**
 * `"7"` → `7`.
 *
 * `undefined` for anything else, including a weak validator (`W/"7"`): the
 * version is used as a precondition, and a weak one means "equivalent, not
 * identical" — precisely the guarantee `If-Match` must not be given.
 */
function parseEtag(etag: string | null): number | undefined {
  if (etag === null) return undefined
  const match = /^"(\d+)"$/.exec(etag)
  if (match === null) return undefined
  return Number(match[1])
}

/** Build a query string, dropping absent values so `?cursor=undefined` cannot happen. */
export function query(params: Record<string, string | number | undefined>): string {
  const search = new URLSearchParams()
  for (const [name, value] of Object.entries(params)) {
    if (value === undefined || value === '') continue
    search.set(name, String(value))
  }
  const encoded = search.toString()
  return encoded === '' ? '' : `?${encoded}`
}

/**
 * A fresh idempotency key.
 *
 * `crypto.randomUUID` is in the ES2022 browser matrix (`docs/18`) and needs a
 * secure context, which every deployment of this app is — the session cookie is
 * `Secure`, so an insecure origin could not authenticate anyway.
 */
export function idempotencyKey(): string {
  return crypto.randomUUID()
}

export type { ErrorEnvelope }
export { ApiError }
