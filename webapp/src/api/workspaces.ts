/**
 * `/api/v1/workspaces` — the tenant the rest of the app is scoped to.
 *
 * # The failure this module prevents
 *
 * A member list rendered as raw UUIDs. Assignees, reporters, and comment
 * authors all arrive as ids; the *only* endpoint that turns an id into a name is
 * `GET /workspaces/{id}/members`. Keeping that call here — beside the type it
 * populates — is what stops each view growing its own half-populated directory.
 */
import { query, request } from './http'
import type { Paged } from './page'

/** `workspaces::wire::WorkspaceBody`. */
export interface Workspace {
  readonly id: string
  readonly name: string
  readonly slug: string
  readonly created_at: string
}

/** `workspaces::wire::MemberBody`. `email` is null once anonymized (ADR-026). */
export interface Member {
  readonly user_id: string
  readonly display_name: string
  readonly email: string | null
  readonly member_type: string
  readonly joined_at: string
}

/**
 * Start a workspace, with the caller as its first member.
 *
 * No workspace header, for the same reason [`listWorkspaces`] sends none: this
 * is the call made *before* there is a tenant to be scoped to. It was the one
 * endpoint the client never called, which left a signed-in person with no
 * workspace looking at "ask an owner for an invitation" and no way to become
 * an owner themselves.
 *
 * `slug` is validated server-side as 1–64 characters of `a-z`, `0-9` and `-`,
 * starting with a letter or digit, and a duplicate is a `409`.
 */
export function createWorkspace(workspace: { name: string; slug: string }): Promise<Workspace> {
  return request<Workspace>('/api/v1/workspaces', { method: 'POST', body: workspace })
}

/**
 * A slug from a name, the way a person would write one.
 *
 * Offered as a default rather than imposed: "Acme, Inc." becoming `acme-inc`
 * is what someone would have typed, and having to type it is a step that only
 * exists because the URL cannot hold a comma. Exported so the form and its test
 * agree on the rule.
 */
export function slugFrom(name: string): string {
  return name
    .toLowerCase()
    .replace(/[^a-z0-9]+/g, '-')
    // Leading and trailing separators are legal characters in the wrong place:
    // the server requires the first character to be a letter or digit.
    .replace(/^-+|-+$/g, '')
    .slice(0, 64)
}

/** Every workspace the caller belongs to. No workspace header — this is the list that picks one. */
export function listWorkspaces(signal?: AbortSignal): Promise<Paged<Workspace>> {
  return request<Paged<Workspace>>(`/api/v1/workspaces${query({ limit: 100 })}`, { signal })
}

export function listMembers(workspaceId: string, signal?: AbortSignal): Promise<Paged<Member>> {
  return request<Paged<Member>>(
    `/api/v1/workspaces/${workspaceId}/members${query({ limit: 100 })}`,
    { workspaceId, signal },
  )
}

/**
 * A lookup from user id to display name.
 *
 * Returns the id itself for anyone not in the page — a task can be reported by
 * someone since removed from the workspace, and rendering "unknown" there would
 * be less true than rendering the id.
 */
export function directory(members: readonly Member[]): (id: string) => string {
  const byId = new Map(members.map((m) => [m.user_id, m.display_name]))
  return (id) => byId.get(id) ?? id
}
