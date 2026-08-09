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
