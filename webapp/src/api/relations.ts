/**
 * Dependencies and subtasks — the two ways a task points at another.
 *
 * # They are different questions and must stay two panels
 *
 * `design/LAYOUT-AND-INTERACTION-GUIDELINES.md` §12: blockers and subtasks are
 * two lists, never one. A blocker gates this task's transitions (ADR-019); a
 * subtask is part of its scope and gates nothing. Merging them into "related
 * work" would put a thing that stops you beside a thing that does not.
 *
 * # `restricted` is an answer, not an error
 *
 * `docs/03`: a blocking task the viewer cannot see "shows as restricted, never
 * as its title". Every field but `restricted` is null together. Dropping those
 * rows would show a task as blocked by nothing — a worse answer than "something
 * you cannot see", because the first one looks actionable.
 */
import { request } from './http'
import type { Task } from './tasks'

/** One end of a relation. All fields but `restricted` are null together. */
export interface Relation {
  readonly id: string | null
  readonly key: string | null
  readonly title: string | null
  readonly state: string | null
  readonly restricted: boolean
}

/** The Relations panel's two lists. */
export interface Relations {
  /** Tasks that must finish before this one may move. */
  readonly blocked_by: readonly Relation[]
  /** Tasks this one is holding up. */
  readonly blocks: readonly Relation[]
}

export function readRelations(
  workspaceId: string,
  taskId: string,
  signal?: AbortSignal,
): Promise<Relations> {
  return request<Relations>(`/api/v1/tasks/${taskId}/dependencies`, { workspaceId, signal })
}

/**
 * Add an edge. Exactly one direction — a request carrying both is a client that
 * has not decided which it means, and the server refuses it.
 *
 * Refused with `TF-TSK-0003` when it would close a loop; the message names the
 * cycle, so the panel can say which link to remove rather than "invalid".
 */
export function addDependency(
  workspaceId: string,
  taskId: string,
  edge: { blocks: string } | { blocked_by: string },
): Promise<Relations> {
  return request<Relations>(`/api/v1/tasks/${taskId}/dependencies`, {
    method: 'POST',
    workspaceId,
    body: edge,
  })
}

/**
 * Remove the edge joining these two tasks, whichever way it points.
 *
 * No direction, because at most one edge can exist between a pair: `A blocks B`
 * and `B blocks A` together are a cycle. The response is the panel's new state,
 * so the caller does not issue a second request for what it just changed.
 */
export function removeDependency(
  workspaceId: string,
  taskId: string,
  otherId: string,
): Promise<Relations> {
  return request<Relations>(`/api/v1/tasks/${taskId}/dependencies/${otherId}`, {
    method: 'DELETE',
    workspaceId,
  })
}

/** The parent, as much of it as a reader needs to navigate to it. */
export interface ParentRef {
  readonly id: string
  readonly key: string
  readonly title: string
  readonly state: string
}

/**
 * A task's place in the tree: one parent above, many children below.
 *
 * # The children are flat, not nested, and that is the wire contract
 *
 * `Relationship` carries `Subtasks` under `#[serde(flatten)]`, so the JSON is
 * `{parent, data, done, total, truncated}` — there is no `children` object.
 * This was typed as nested first, and the stub in the panel test repeated the
 * same assumption, so the test passed and the surface crashed on the first real
 * response. The shape here is copied from the payload, not from the Rust struct
 * names.
 */
export interface Relationship {
  /** `null` when this task is not a subtask, which is most tasks. */
  readonly parent: ParentRef | null
  readonly data: readonly Task[]
  /**
   * Counted server-side across every child the caller may see — not from `data`,
   * which may be one page of them.
   *
   * **Displayed, never enforced.** Nothing offers to close a parent because
   * `done === total`, and no endpoint would accept it.
   */
  readonly done: number
  readonly total: number
  /** True when there are more children than one read returns. */
  readonly truncated: boolean
}

export function readSubtasks(
  workspaceId: string,
  taskId: string,
  signal?: AbortSignal,
): Promise<Relationship> {
  return request<Relationship>(`/api/v1/tasks/${taskId}/subtasks`, { workspaceId, signal })
}
