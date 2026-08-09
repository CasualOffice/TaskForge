/**
 * `/api/v1/roles` and `/api/v1/role-assignments` — authority, as data.
 *
 * # The two verbs are two permissions, and that is not a detail
 *
 * `docs/04` (D-049) splits authoring a role (`role.manage`) from granting one
 * (`role.assign`). A screen that treats them as one "admin" flag will offer the
 * editor to someone who may only assign, and their first save is a `403`. So
 * every function here says which permission it needs, and the screens ask
 * `useAuthority` for that exact key.
 *
 * # A grant is a row, not a checkbox
 *
 * `{principal, role, scope}` — a role given to a user or a team, at a workspace,
 * team, project or environment. Revoking needs the **assignment id**, which is
 * why [`listAssignments`] exists: without it the id appeared once, in the
 * response to the call that created it, and authority could be given but never
 * taken back.
 */
import { query, request } from './http'
import type { Paged } from './page'

export interface Role {
  readonly id: string
  readonly name: string
  /** A cloneable starting point (`docs/04`). Editable like any other role. */
  readonly is_template: boolean
  readonly permissions: readonly string[]
  readonly created_at: string
  readonly updated_at: string
  readonly version: number
}

export type PrincipalType = 'USER' | 'TEAM'
export type ScopeType = 'WORKSPACE' | 'TEAM' | 'PROJECT' | 'ENVIRONMENT'

export interface Assignment {
  readonly id: string
  readonly principal_type: PrincipalType
  readonly principal_id: string
  readonly role_id: string
  readonly scope_type: ScopeType
  readonly scope_id: string
  readonly granted_by: string
  readonly granted_at: string
}

/** Every role in the workspace. Not paged — an admin-authored set (`docs/21`). */
export function listRoles(
  workspaceId: string,
  signal?: AbortSignal,
): Promise<{ data: readonly Role[] }> {
  return request<{ data: readonly Role[] }>('/api/v1/roles', { workspaceId, signal })
}

/** Needs `role.manage`. Refused with `TF-AZN-0003` if it carries what you lack. */
export function createRole(
  workspaceId: string,
  role: { name: string; permissions: readonly string[] },
): Promise<Role> {
  return request<Role>('/api/v1/roles', { method: 'POST', workspaceId, body: role })
}

/**
 * Rename, or replace the permission set.
 *
 * `permissions` is a **replacement**, never a merge: `docs/04` control 1
 * re-checks the grant ceiling on edit, and a merge would let a permission
 * survive that the check never saw.
 */
export function updateRole(
  workspaceId: string,
  id: string,
  version: number,
  patch: { name?: string; permissions?: readonly string[] },
): Promise<Role> {
  return request<Role>(`/api/v1/roles/${id}`, {
    method: 'PATCH',
    workspaceId,
    ifMatch: version,
    body: patch,
  })
}

/**
 * Who holds what, and where. Every filter narrows; none is required.
 *
 * `scope_id` alone identifies the scope — an id names exactly one thing, so a
 * `scope_type` beside it could only be a way to disagree with it.
 */
export function listAssignments(
  workspaceId: string,
  filter: {
    principalId?: string | undefined
    roleId?: string | undefined
    scopeId?: string | undefined
    cursor?: string | undefined
  } = {},
  signal?: AbortSignal,
): Promise<Paged<Assignment>> {
  return request<Paged<Assignment>>(
    `/api/v1/role-assignments${query({
      principal_id: filter.principalId,
      role_id: filter.roleId,
      scope_id: filter.scopeId,
      cursor: filter.cursor,
      limit: 100,
    })}`,
    { workspaceId, signal },
  )
}

/**
 * Grant a role. Needs `role.assign`.
 *
 * `scopeId` omitted means the workspace itself. The refusals are the ceilings in
 * `docs/04` and each has its own code, so a screen can say which rule was hit
 * rather than "denied": `TF-AZN-0003` (you do not hold it), `TF-AZN-0004` (above
 * your scope), `TF-AZN-0006` (you cannot give it to yourself).
 */
export function assignRole(
  workspaceId: string,
  grant: {
    principal_type: PrincipalType
    principal_id: string
    role_id: string
    scope_type: ScopeType
    scope_id?: string | undefined
  },
): Promise<Assignment> {
  return request<Assignment>('/api/v1/role-assignments', {
    method: 'POST',
    workspaceId,
    body: grant,
  })
}

/** Take a grant back. The id comes from [`listAssignments`]. */
export function revokeAssignment(workspaceId: string, id: string): Promise<void> {
  return request<void>(`/api/v1/role-assignments/${id}`, { method: 'DELETE', workspaceId })
}

/**
 * Every permission key this build knows, grouped the way `docs/04` groups them.
 *
 * Typed out rather than fetched because there is no endpoint that lists them —
 * `permission(key)` is a table the server validates against, and an unknown key
 * is refused with `TF-VAL-0005`. If this list drifts from the server's, the
 * symptom is that refusal on save, naming the key, which is a legible failure.
 */
export const PERMISSION_GROUPS: ReadonlyArray<{
  readonly title: string
  readonly keys: readonly string[]
}> = [
  {
    title: 'Tasks',
    keys: [
      'task.read',
      'task.create',
      'task.update',
      'task.delete',
      'task.assign',
      'task.move',
      'task.transition',
      'task.close',
      'task.reopen',
      'task.comment',
      'task.history.read',
      'task.dependency.override',
      'task.attachment.create',
      'task.attachment.read',
    ],
  },
  {
    title: 'Projects',
    keys: [
      'project.create',
      'project.update',
      'project.delete',
      'project.member.manage',
      'project.role.assign',
      'project.workflow.manage',
    ],
  },
  {
    title: 'Workspace',
    keys: [
      'workspace.manage',
      'workspace.delete',
      'workspace.owner',
      'tag.manage',
      'role.assign',
      'role.manage',
      'audit.read',
      'plugin.install',
      'automation.manage',
    ],
  },
]

/** Every key the groups above carry, flattened. The editor renders from this. */
export const ALL_PERMISSIONS: readonly string[] = PERMISSION_GROUPS.flatMap((g) => g.keys)

/**
 * A short gloss for a permission key.
 *
 * Keys are legible to whoever wrote the model and opaque to an admin choosing
 * one: `task.move` and `task.transition` are both "change where a task is" until
 * someone says which is which. Absent keys fall back to the key itself rather
 * than to an invented sentence — a wrong description of an authority is worse
 * than none.
 */
export const PERMISSION_HELP: Readonly<Record<string, string>> = {
  'task.read': 'See tasks at all. Without it, nothing else here matters.',
  'task.create': 'Raise new tasks.',
  'task.update': 'Edit a task\u2019s fields \u2014 title, description, dates, priority.',
  'task.delete': 'Delete a task.',
  'task.assign': 'Put people on a task, or take them off.',
  'task.move': 'Move a task to a different project.',
  'task.transition': 'Move a task through the workflow.',
  'task.close': 'Take the edge into a completed status.',
  'task.reopen': 'Take a completed task back out.',
  'task.comment': 'Write comments.',
  'task.history.read': 'Read the activity trail on a task.',
  'task.dependency.override': 'Transition a task that is still blocked.',
  'task.attachment.create': 'Attach files.',
  'task.attachment.read': 'Download attachments.',
  'project.create': 'Create projects.',
  'project.update': 'Rename a project and change its settings.',
  'project.delete': 'Delete a project.',
  'project.member.manage': 'Add and remove project members.',
  'project.role.assign': 'Grant roles at project scope.',
  'project.workflow.manage': 'Author statuses and transitions.',
  'workspace.manage': 'Rename the workspace and change its settings.',
  'workspace.delete': 'Delete the whole workspace.',
  'workspace.owner': 'Everything. The last holder cannot be removed.',
  'tag.manage': 'Author the tag vocabulary.',
  'role.assign': 'Grant existing roles to people and teams.',
  'role.manage': 'Author the roles themselves. Workspace scope only.',
  'audit.read': 'Read the audit trail.',
  'plugin.install': 'Install plugins.',
  'automation.manage': 'Author automation rules.',
}
