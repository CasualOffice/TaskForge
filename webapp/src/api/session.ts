/**
 * `/api/v1/auth/*` — who the caller is, and how they stop being it.
 *
 * # The failure this module prevents
 *
 * Treating "signed in" and "may enter this workspace" as one question.
 * `crates/casual-task-api/src/middleware.rs` splits them into two extractors on
 * purpose: the session is user-scoped, membership is per workspace. The client
 * mirrors that split — [`readSession`] sends no workspace header, because
 * answering "who am I" must not require having chosen a workspace first. A
 * client that asked with a workspace would log a user out whenever their last
 * workspace was one they had been removed from.
 */
import { request, requestNoContent } from './http'
import { ApiError } from './problem'

/** `GET /api/v1/auth/session` — the body `middleware::whoami` returns. */
export interface SessionView {
  readonly actor_id: string
  readonly actor_type: string
}

/** `POST /api/v1/auth/login` — the body `auth::login` returns on success. */
export interface LoginResponse {
  readonly csrf_token: string
}

export interface Credentials {
  readonly email: string
  readonly password: string
}

/**
 * Who the caller is, or `null` when nobody.
 *
 * A 401 is a *state*, not an error: it is the answer for every visitor who has
 * not signed in yet. Throwing here would put the sign-in screen behind an error
 * boundary, which is why the auth failure is caught and folded into the value.
 */
export async function readSession(signal?: AbortSignal): Promise<SessionView | null> {
  try {
    return await request<SessionView>('/api/v1/auth/session', { signal })
  } catch (error) {
    if (isUnauthenticated(error)) return null
    throw error
  }
}

/**
 * Exchange credentials for a session.
 *
 * The response's `csrf_token` is deliberately ignored: the server sets the same
 * value in the readable `tf_csrf` cookie, and `http.ts` reads it from there on
 * every request. Keeping it in memory as well would create a second copy that
 * goes stale the moment the session is renewed in another tab.
 */
export async function login(credentials: Credentials): Promise<void> {
  await request<LoginResponse>('/api/v1/auth/login', { method: 'POST', body: credentials })
}

/** Revoke the session row. `docs/40` rejects JWTs precisely so this is immediate. */
export async function logout(): Promise<void> {
  await requestNoContent('/api/v1/auth/logout', { method: 'POST' })
}

function isUnauthenticated(error: unknown): boolean {
  return error instanceof ApiError && error.status === 401
}
