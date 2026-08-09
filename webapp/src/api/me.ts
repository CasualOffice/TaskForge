/**
 * `/api/v1/me` — the signed-in person, independent of any workspace.
 *
 * # Why this is the one client module that sends no workspace
 *
 * `user_account` is the only table without a `workspace_id` (`docs/32`), because
 * a person belongs to many workspaces and their name is not a fact about any one
 * of them. Every other call in `api/` takes a `workspaceId` first precisely so it
 * cannot be forgotten; these take none, and that asymmetry is the contract rather
 * than an oversight.
 *
 * # No `If-Match`
 *
 * The other mutable aggregates carry a version because two people edit them. An
 * account has exactly one editor, so `docs/24`'s lost-update problem does not
 * arise and the server requires no precondition. A client that invented one
 * would be sending a header the endpoint refuses to understand.
 */
import { request } from './http'

/** `me::MeView`. `email` is `null` once the account is anonymized (ADR-026). */
export interface Me {
  readonly id: string
  readonly email: string | null
  readonly display_name: string
  readonly avatar_url: string | null
  /** An IANA zone name, or `null` — which is **not** the same as UTC. */
  readonly time_zone: string | null
}

/**
 * One live session.
 *
 * `current` is the server's answer, not a comparison the client makes: the
 * session id a browser holds is inside an `HttpOnly` cookie it cannot read, so
 * a client deciding this itself would have to guess.
 */
export interface LiveSession {
  readonly id: string
  readonly auth_method: string
  readonly created_at: string
  readonly last_seen_at: string
  readonly expires_at: string
  readonly ip_address: string | null
  readonly user_agent: string | null
  readonly current: boolean
}

export function readMe(signal?: AbortSignal): Promise<Me> {
  return request<Me>('/api/v1/me', { signal })
}

/**
 * Change the name, the zone, or both.
 *
 * `time_zone: null` **clears** it and `undefined` leaves it alone — `docs/05`
 * §Conventions, and the reason the parameter is typed with `null` in the union
 * rather than being optional-only. Clearing means "use UTC"; leaving it alone
 * means "I did not touch this".
 */
export function updateMe(patch: {
  display_name?: string
  time_zone?: string | null
}): Promise<Me> {
  return request<Me>('/api/v1/me', { method: 'PATCH', body: patch })
}

/**
 * Change the password.
 *
 * Every other session is refused afterwards — that is migration 0016's
 * `changed_at` rule and it applies to the caller's own other tabs too, so a
 * screen that offers this must say so before it is pressed rather than explain
 * it afterwards.
 */
export function changePassword(current: string, next: string): Promise<void> {
  return request<void>('/api/v1/me/password', {
    method: 'POST',
    body: { current_password: current, new_password: next },
  })
}

export function listSessions(signal?: AbortSignal): Promise<{ data: readonly LiveSession[] }> {
  return request<{ data: readonly LiveSession[] }>('/api/v1/me/sessions', { signal })
}

export function revokeSession(id: string): Promise<void> {
  return request<void>(`/api/v1/me/sessions/${id}`, { method: 'DELETE' })
}

/** Sign out everywhere else, keeping this one. The "I lost my laptop" button. */
export function revokeOtherSessions(): Promise<void> {
  return request<void>('/api/v1/me/sessions', { method: 'DELETE' })
}
