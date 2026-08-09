/**
 * `can(...)` — the one question a component asks about authority.
 *
 * # The failure this module prevents
 *
 * Two components disagreeing about the same person. docs/42 §Permissions in the
 * UI sketches exactly this hook, and the reason it is a hook rather than a prop
 * threaded down is that a prop gets forgotten on the third screen and someone
 * writes `role === 'admin'` instead.
 *
 * # What `can` returns while the answer is unknown
 *
 * `false`. Optimistically showing a control and removing it a moment later is
 * worse than showing it a moment late: a button that vanishes under the cursor
 * is a bug report, and a button that appears is not.
 *
 * # What it does with `conditional`
 *
 * Treats it as permission. A conditional grant means "where the constraints
 * hold", and only the server can evaluate those for a given task — so the
 * control is offered and a refusal, if it comes, is rendered from its registry
 * code. Hiding conditional permissions would hide the reporter's own Close
 * button on the task they reported, which is precisely the case the constraint
 * exists to allow.
 */
import { useQuery } from '@tanstack/react-query'

import { keys } from '../api/keys'
import { readEffective, type Reach } from '../api/permissions'
import { useWorkspaceId } from './session'

export interface Authority {
  /** Whether the actor may do this here. `false` until the answer arrives. */
  readonly can: (permission: string) => boolean
  /** `undefined` when absent — lets a caller distinguish the two grant kinds. */
  readonly reachOf: (permission: string) => Reach | undefined
  /**
   * The task types the actor may raise here, or `undefined` for "no narrowing".
   *
   * `undefined` and an empty array are different answers: the first is every
   * type, the second is none — which is what a grant narrowed to types the
   * project does not use looks like, and it must not read as "all".
   */
  readonly creatableTypes: readonly string[] | undefined
  readonly loading: boolean
}

/**
 * The actor's effective permissions, workspace-wide or in one project.
 *
 * Cached for five minutes and invalidated by nothing yet: `docs/42` says the set
 * is invalidated by the `authz_epoch` bump arriving over SSE, "so a revoked
 * permission disappears from the UI within a second rather than at next reload".
 * The stream exists (C-015) but does not carry that event, so today the window
 * is the stale time. Recorded rather than left implicit — a permission that
 * lingers for five minutes in a menu is a UI fact, not a security one, because
 * the server re-authorizes every mutation.
 */
export function useAuthority(projectId?: string): Authority {
  const workspaceId = useWorkspaceId()

  const result = useQuery({
    queryKey: [...keys.workspace(workspaceId), 'permissions', projectId ?? ''],
    queryFn: ({ signal }) =>
      readEffective(workspaceId, projectId === undefined ? {} : { projectId }, signal),
    enabled: workspaceId !== '',
    staleTime: 5 * 60_000,
  })

  const entries = result.data?.permissions ?? []
  const byKey = new Map(entries.map((entry) => [entry.permission, entry.reach]))

  return {
    can: (permission) => byKey.has(permission),
    reachOf: (permission) => byKey.get(permission),
    creatableTypes: entries.find((entry) => entry.permission === 'task.create')?.task_types,
    loading: result.isPending,
  }
}
