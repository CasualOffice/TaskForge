/**
 * Workspace administration: the workspace itself, its people, its teams.
 *
 * # Membership is not authority
 *
 * Migration 0003: "role_assignment is the ONLY source of authority in the
 * system." Adding someone to a workspace makes them *visible* — it grants
 * nothing. A settings screen that treated "add member" as onboarding would
 * produce accounts that can sign in and see an empty product, so the member and
 * role screens are deliberately separate and the member list shows what each
 * person actually holds.
 *
 * # A team is a principal, not a folder
 *
 * `docs/04` lets a grant name a team, and everyone in it inherits. That is the
 * only reason teams exist, and it is why the team screen shows membership: "who
 * does this grant reach?" is otherwise unanswerable.
 */
import { query, request, requestNoContent } from './http'
import type { Paged } from './page'
import type { Member, Workspace } from './workspaces'

export type { Member, Workspace }

/** `workspaces::wire::TeamBody`. */
export interface Team {
  readonly id: string
  readonly name: string
  readonly created_at: string
}

/**
 * `workspaces::wire::TeamMemberBody`.
 *
 * No `joined_at`: `team_membership` is `(team_id, user_id)` and nothing else
 * (migration 0002). The workspace's join date was the available candidate and
 * answers a different question.
 */
export interface TeamMember {
  readonly user_id: string
  readonly display_name: string
  readonly email: string | null
  /** Their standing in the **workspace** — enough to mark a guest in a team. */
  readonly member_type: string
}

/** `invitations::InvitationBody`. */
export interface Invitation {
  readonly id: string
  readonly email: string
  readonly role_id: string | null
  readonly invited_by: string | null
  readonly expires_at: string
  readonly created_at: string
}

export function readWorkspace(workspaceId: string, signal?: AbortSignal): Promise<Workspace> {
  return request<Workspace>(`/api/v1/workspaces/${workspaceId}`, { workspaceId, signal })
}

/**
 * Rename. Conditional on the version, because two admins can hold the settings
 * page open at once — which is exactly the lost update `docs/24` exists for.
 *
 * The slug is **not** renameable: it is in every URL anyone has shared.
 */
export function renameWorkspace(
  workspaceId: string,
  version: number,
  name: string,
): Promise<Workspace> {
  return request<Workspace>(`/api/v1/workspaces/${workspaceId}`, {
    method: 'PATCH',
    workspaceId,
    ifMatch: version,
    body: { name },
  })
}

export function listMembers(
  workspaceId: string,
  cursor?: string,
  signal?: AbortSignal,
): Promise<Paged<Member>> {
  return request<Paged<Member>>(
    `/api/v1/workspaces/${workspaceId}/members${query({ limit: 100, cursor })}`,
    { workspaceId, signal },
  )
}

/**
 * Remove someone from the workspace.
 *
 * Refused with `TF-PRJ-0006` if they are the last member and `TF-AZN-0005` if
 * they hold the last `workspace.owner` grant — the second is a database trigger
 * (migration 0021), not a check this client can pre-empt, so the screen offers
 * the control and renders the refusal.
 */
export function removeMember(workspaceId: string, userId: string): Promise<void> {
  return requestNoContent(`/api/v1/workspaces/${workspaceId}/members/${userId}`, {
    method: 'DELETE',
    workspaceId,
  })
}

export function listInvitations(
  workspaceId: string,
  signal?: AbortSignal,
): Promise<Paged<Invitation>> {
  return request<Paged<Invitation>>(
    `/api/v1/workspaces/${workspaceId}/invitations${query({ limit: 100 })}`,
    { workspaceId, signal },
  )
}

/**
 * Invite someone by email, optionally with the role they will hold on arrival.
 *
 * The role is checked against the inviter's own ceiling *now* (`docs/04`
 * control 1), so an invitation cannot be a way to grant what you could not
 * grant directly.
 */
export function createInvitation(
  workspaceId: string,
  invite: { email: string; role_id?: string | undefined },
): Promise<Invitation> {
  return request<Invitation>(`/api/v1/workspaces/${workspaceId}/invitations`, {
    method: 'POST',
    workspaceId,
    body: invite,
  })
}

export function revokeInvitation(workspaceId: string, id: string): Promise<void> {
  return requestNoContent(`/api/v1/workspaces/${workspaceId}/invitations/${id}`, {
    method: 'DELETE',
    workspaceId,
  })
}

/**
 * The teams the signed-in person is in.
 *
 * A different endpoint from [`listTeams`], not a filter on it, because they
 * answer different questions: that one is an administrator choosing a team out
 * of everything the workspace has, this one is a person finding their own work.
 * A sidebar built on the administrative list would grow with the workspace
 * rather than with the person.
 */
export function listMyTeams(workspaceId: string, signal?: AbortSignal): Promise<Paged<Team>> {
  return request<Paged<Team>>('/api/v1/me/teams', { workspaceId, signal })
}

export function listTeams(workspaceId: string, signal?: AbortSignal): Promise<Paged<Team>> {
  return request<Paged<Team>>(`/api/v1/workspaces/${workspaceId}/teams${query({ limit: 100 })}`, {
    workspaceId,
    signal,
  })
}

export function createTeam(workspaceId: string, name: string): Promise<Team> {
  return request<Team>(`/api/v1/workspaces/${workspaceId}/teams`, {
    method: 'POST',
    workspaceId,
    body: { name },
  })
}

export function listTeamMembers(
  workspaceId: string,
  teamId: string,
  signal?: AbortSignal,
): Promise<Paged<TeamMember>> {
  return request<Paged<TeamMember>>(`/api/v1/teams/${teamId}/members${query({ limit: 100 })}`, {
    workspaceId,
    signal,
  })
}

/**
 * Put a workspace member in a team.
 *
 * Refused with `TF-VAL-0007` for someone who is not a member of this workspace:
 * `team_membership` carries no `workspace_id` and therefore no policy of its own
 * (migration 0010), so that check is the tenant boundary rather than a nicety.
 */
export function addTeamMember(
  workspaceId: string,
  teamId: string,
  userId: string,
): Promise<unknown> {
  return request<unknown>(`/api/v1/teams/${teamId}/members`, {
    method: 'POST',
    workspaceId,
    body: { user_id: userId },
  })
}

export function removeTeamMember(
  workspaceId: string,
  teamId: string,
  userId: string,
): Promise<void> {
  return requestNoContent(`/api/v1/teams/${teamId}/members/${userId}`, {
    method: 'DELETE',
    workspaceId,
  })
}
