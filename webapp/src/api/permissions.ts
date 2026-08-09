/**
 * `GET /api/v1/permissions/effective` — what the actor may do here.
 *
 * # The failure this module prevents
 *
 * Affordances hard-coded into components. Every `{isAdmin && <Button/>}` is a
 * second, divergent copy of `docs/04`'s resolver written in TypeScript by
 * whoever needed a button that day — and the copies drift, so one screen offers
 * an action another screen hides for the same person.
 *
 * The server already resolves authority; this asks it. `docs/04`'s own note on
 * why answering is safe: "Telling someone they lack a permission discloses
 * nothing they could not learn by pressing the button."
 *
 * # `reach` is a three-way answer, not a boolean
 *
 * `unconditional` means the permission holds on every resource in the scope.
 * `conditional` means it holds where the grant's constraints do — `task.close`
 * granted only for tasks you reported, say. A client that treated the two the
 * same would either hide a control the user can use, or offer one that refuses.
 * Absent is the third answer.
 *
 * **Hiding a control is presentation, never security** (docs/42 §Permissions in
 * the UI). The server re-authorizes every mutation regardless.
 */
import { query, request } from './http'

export type Reach = 'unconditional' | 'conditional'

export interface EffectivePermission {
  readonly permission: string
  readonly reach: Reach
}

export interface EffectivePermissions {
  readonly workspace_id: string
  readonly actor_id: string
  readonly project_id: string | null
  readonly permissions: readonly EffectivePermission[]
}

/**
 * The caller's own permissions, workspace-wide or narrowed to a project.
 *
 * `team_id` is passed through when the project has one: a team-scoped grant only
 * reaches a project through its team, so omitting a real team **understates**
 * the answer — and an understated answer hides a control the user could use.
 */
export function readEffective(
  workspaceId: string,
  scope: { projectId?: string | undefined; teamId?: string | undefined } = {},
  signal?: AbortSignal,
): Promise<EffectivePermissions> {
  return request<EffectivePermissions>(
    `/api/v1/permissions/effective${query({
      project_id: scope.projectId,
      team_id: scope.teamId,
    })}`,
    { workspaceId, signal },
  )
}

/** The permission keys this client asks about. Copied from `casual-task-model`. */
export const PERMISSIONS = {
  taskCreate: 'task.create',
  taskUpdate: 'task.update',
  taskDelete: 'task.delete',
  taskAssign: 'task.assign',
  taskTransition: 'task.transition',
  taskComment: 'task.comment',
  taskHistoryRead: 'task.history.read',
  taskAttachmentRead: 'task.attachment.read',
  projectCreate: 'project.create',
} as const
