/**
 * Server refusals, as something a user can act on.
 *
 * # The failure this module prevents
 *
 * Rendering a raw response body. `docs/05` §Errors gives every refusal a stable
 * code from the `docs/20` registry precisely so a client never has to show the
 * user a status line or a stack — and the moment one call site does
 * `catch (e) { toast(String(e)) }`, that promise is worth nothing. So the parse
 * lives here, the code→sentence table lives here, and `ApiError` is the only
 * error shape the rest of the app ever sees.
 *
 * # What is deliberately *not* here
 *
 * A message for every code in `docs/20`. A code the client cannot meet — an
 * internal fault, an unknown filter field the user never typed — gets the
 * generic sentence plus its `request_id`, which is the thing support actually
 * asks for. Inventing reassuring prose for `TF-SYS-0001` would be inventing a
 * diagnosis.
 */

/** The envelope `crates/casual-task-api/src/error.rs` serializes. */
export interface ErrorEnvelope {
  readonly error: {
    readonly code: string
    readonly message: string
    readonly details?: unknown
    readonly request_id: string
    readonly docs: string
  }
}

/** What the user can do about a refusal. Decides which UI affordance appears. */
export type Remedy =
  /** Sign in again. The credential is gone or was never valid. */
  | 'reauthenticate'
  /** Nothing — the actor lacks the grant. Hide or disable the control. */
  | 'forbidden'
  /** Fix the input and resubmit. */
  | 'fix-input'
  /** Someone else wrote first; reload and reapply. */
  | 'conflict'
  /** Wait and retry. Rate limit or shed load. */
  | 'retry-later'
  /** The thing is not there, or not visible. Navigate away. */
  | 'gone'
  /** Report it. Nothing the user does changes the outcome. */
  | 'report'

/** The code the transport uses when the request never reached a server. */
export const OFFLINE = 'TF-NET-OFFLINE'

/**
 * Codes this client renders a specific sentence for.
 *
 * Every key is copied from `crates/casual-task-api/src/error.rs`, which is
 * itself copied from `docs/20`. A code invented here would be a code no server
 * sends and no user could look up.
 */
const KNOWN: Readonly<Record<string, { remedy: Remedy; sentence: string }>> = {
  [OFFLINE]: {
    remedy: 'retry-later',
    sentence: 'TaskForge could not be reached. Check your connection and try again.',
  },
  'TF-AUT-0001': { remedy: 'reauthenticate', sentence: 'Your session has ended. Sign in again.' },
  'TF-AUT-0008': {
    remedy: 'reauthenticate',
    sentence: 'This tab’s security token is stale. Reload the page and try again.',
  },
  'TF-AUT-0013': {
    remedy: 'reauthenticate',
    sentence: 'That credential cannot be used here. Sign in with your account.',
  },
  'TF-AZN-0001': {
    remedy: 'forbidden',
    sentence: 'You do not have permission to do that.',
  },
  'TF-AZN-0002': {
    remedy: 'forbidden',
    sentence: 'Your permission does not cover this item.',
  },
  'TF-AZN-0005': {
    remedy: 'forbidden',
    sentence: 'A workspace must keep at least one owner.',
  },
  'TF-AZN-0008': { remedy: 'gone', sentence: 'That is not available.' },
  'TF-PRJ-0001': { remedy: 'gone', sentence: 'That project is not available.' },
  'TF-PRJ-0002': { remedy: 'fix-input', sentence: 'That project key is already in use.' },
  'TF-PRJ-0003': { remedy: 'fix-input', sentence: 'A project key cannot be changed once set.' },
  'TF-PRJ-0004': {
    remedy: 'fix-input',
    sentence: 'A project key is 2–10 characters: a capital letter, then capitals or digits.',
  },
  'TF-TSK-0001': { remedy: 'gone', sentence: 'That task is not available.' },
  'TF-TSK-0005': {
    remedy: 'fix-input',
    sentence: 'That person is not a member of this project.',
  },
  'TF-TSK-0006': {
    remedy: 'fix-input',
    sentence: 'A parent task must be in the same project, and subtasks are one level deep.',
  },
  'TF-WFL-0001': {
    remedy: 'report',
    sentence: 'Status is changed by a transition, not by editing the field.',
  },
  'TF-WFL-0002': {
    remedy: 'fix-input',
    sentence: 'This workflow has no move from the current status to that one.',
  },
  'TF-WFL-0003': {
    remedy: 'forbidden',
    sentence: 'You do not have permission to make that transition.',
  },
  'TF-WFL-0004': {
    remedy: 'fix-input',
    sentence: 'That status needs fields this task does not have yet.',
  },
  'TF-WFL-0005': {
    remedy: 'fix-input',
    sentence: 'This task is blocked by a dependency that is not resolved.',
  },
  'TF-CNC-0001': {
    remedy: 'conflict',
    sentence: 'Someone else changed this first. Your edit was not applied.',
  },
  'TF-CNC-0002': { remedy: 'report', sentence: 'This edit was sent without a version to check against.' },
  'TF-CNC-0003': { remedy: 'report', sentence: 'This edit sent a version the server could not read.' },
  'TF-IDM-0001': {
    remedy: 'retry-later',
    sentence: 'That request is already being processed. Give it a moment.',
  },
  'TF-IDM-0002': {
    remedy: 'report',
    sentence: 'This request reused a key with different content.',
  },
  'TF-IDM-0003': { remedy: 'report', sentence: 'This request was sent without an idempotency key.' },
  'TF-VAL-0001': { remedy: 'fix-input', sentence: 'The server could not read that request.' },
  'TF-VAL-0002': { remedy: 'report', sentence: 'This request sent a field the server does not accept.' },
  'TF-VAL-0003': { remedy: 'fix-input', sentence: 'A required field is missing.' },
  'TF-VAL-0004': { remedy: 'fix-input', sentence: 'A value is outside the range the server accepts.' },
  'TF-VAL-0005': { remedy: 'fix-input', sentence: 'A value is not one of the allowed options.' },
  'TF-VAL-0007': { remedy: 'fix-input', sentence: 'This refers to something that does not exist.' },
  'TF-QRY-0001': { remedy: 'fix-input', sentence: 'That filter names a field you cannot filter on.' },
  'TF-QRY-0002': { remedy: 'fix-input', sentence: 'That column cannot be sorted on.' },
  'TF-QRY-0003': { remedy: 'fix-input', sentence: 'That comparison does not work on that field.' },
  'TF-QRY-0004': { remedy: 'fix-input', sentence: 'That filter has too many conditions.' },
  'TF-QRY-0005': { remedy: 'fix-input', sentence: 'That filter is nested too deeply.' },
  'TF-QRY-0006': {
    remedy: 'report',
    sentence: 'This page link has expired. Go back to the first page.',
  },
  'TF-QRY-0007': { remedy: 'report', sentence: 'This request asked for too many rows at once.' },
  'TF-QRY-0008': { remedy: 'fix-input', sentence: 'Search text is limited to 256 characters.' },
  'TF-LIM-0001': {
    remedy: 'retry-later',
    sentence: 'Too many requests. Wait a moment and try again.',
  },
  'TF-SYS-0002': {
    remedy: 'retry-later',
    sentence: 'TaskForge is briefly unavailable. Try again shortly.',
  },
}

const GENERIC = 'Something went wrong on the server. Nothing you sent caused it.'

/**
 * A refusal from the API, or a request that never got one.
 *
 * Carries the `request_id` because `docs/05` promises the user something to
 * quote to support, and an error object that dropped it makes that promise
 * false in exactly the situation it exists for.
 */
export class ApiError extends Error {
  readonly code: string
  readonly status: number
  readonly requestId: string | undefined
  readonly details: unknown
  readonly retryAfterSeconds: number | undefined
  readonly docs: string | undefined
  /**
   * Whether the response carried the `docs/05` error envelope.
   *
   * `false` distinguishes a refusal the *application* made from one the router
   * made: a 404 with `TF-AZN-0008` means "absent or invisible", while a 404 with
   * no envelope means the route is not registered at all. Both are 404s and only
   * one of them is about the caller's request — see `api/workflow.ts`, where
   * telling them apart is the difference between "you cannot see this project"
   * and "this server does not serve workflows yet".
   */
  readonly hasEnvelope: boolean

  constructor(init: {
    code: string
    status: number
    message: string
    requestId?: string | undefined
    details?: unknown
    retryAfterSeconds?: number | undefined
    docs?: string | undefined
    hasEnvelope?: boolean
    cause?: unknown
  }) {
    super(init.message, init.cause === undefined ? undefined : { cause: init.cause })
    this.name = 'ApiError'
    this.code = init.code
    this.status = init.status
    this.requestId = init.requestId
    this.details = init.details
    this.retryAfterSeconds = init.retryAfterSeconds
    this.docs = init.docs
    this.hasEnvelope = init.hasEnvelope ?? false
  }

  /** The request never reached a server — offline, DNS, or a blocked origin. */
  static offline(cause: unknown): ApiError {
    return new ApiError({ code: OFFLINE, status: 0, message: 'network unreachable', cause })
  }

  /**
   * A success with no body, on a route whose caller was promised one.
   *
   * `TF-SYS-0001` rather than a new code: `docs/20` owns the registry, this is
   * a server fault the user did not cause, and that is precisely what the
   * generic system code already says. Inventing `TF-SYS-0003` here would put a
   * code on screen that the error catalogue has never heard of.
   */
  static emptyBody(path: string, status: number): ApiError {
    return new ApiError({ code: 'TF-SYS-0001', status, message: `empty body from ${path}` })
  }

  /**
   * Parse a non-2xx response.
   *
   * Tolerates a body that is not the envelope: a reverse proxy returning its own
   * 502 HTML is exactly when a client must not crash on `JSON.parse`.
   */
  static async fromResponse(response: Response): Promise<ApiError> {
    const retryAfter = Number(response.headers.get('retry-after') ?? '')
    const requestIdHeader = response.headers.get('x-request-id') ?? undefined
    let envelope: Partial<ErrorEnvelope['error']> = {}
    try {
      const parsed: unknown = await response.json()
      if (typeof parsed === 'object' && parsed !== null && 'error' in parsed) {
        envelope = (parsed as ErrorEnvelope).error
      }
    } catch {
      // Not the envelope. Fall through to status-derived defaults.
    }
    return new ApiError({
      code: envelope.code ?? fallbackCode(response.status),
      status: response.status,
      message: envelope.message ?? response.statusText,
      requestId: envelope.request_id ?? requestIdHeader,
      details: envelope.details,
      retryAfterSeconds: Number.isFinite(retryAfter) && retryAfter > 0 ? retryAfter : undefined,
      docs: envelope.docs,
      hasEnvelope: envelope.code !== undefined,
    })
  }

  /** What the user can do about it. */
  get remedy(): Remedy {
    return KNOWN[this.code]?.remedy ?? (this.status >= 500 ? 'report' : 'fix-input')
  }

  /** A sentence to show the user. Never the server's own message. */
  get sentence(): string {
    return KNOWN[this.code]?.sentence ?? GENERIC
  }

  /** Whether the shell should drop to the sign-in screen. */
  get isAuthFailure(): boolean {
    return this.remedy === 'reauthenticate'
  }
}

/** A status with no envelope still gets a registry code, not an invented one. */
function fallbackCode(status: number): string {
  if (status === 401) return 'TF-AUT-0001'
  if (status === 403) return 'TF-AZN-0001'
  if (status === 404) return 'TF-AZN-0008'
  if (status === 409) return 'TF-CNC-0001'
  if (status === 429) return 'TF-LIM-0001'
  if (status === 503) return 'TF-SYS-0002'
  return 'TF-SYS-0001'
}

/** Narrow an unknown thrown value. Every `catch` in the app goes through this. */
export function asApiError(error: unknown): ApiError {
  if (error instanceof ApiError) return error
  return new ApiError({
    code: 'TF-SYS-0001',
    status: 0,
    message: error instanceof Error ? error.message : String(error),
    cause: error,
  })
}
