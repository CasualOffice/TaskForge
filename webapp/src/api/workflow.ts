/**
 * `/api/v1/workflows/{id}` — the board's columns, and the workflow editor.
 *
 * # Two consumers, one shape
 *
 * A board needs the status set to draw columns and the transition set to know
 * which drags are legal before the drop. The workflow editor needs the same two
 * lists, plus the writes. They are one module because a second read shape would
 * be a second thing to keep in step with `docs/23`.
 *
 * # `from: null` is the initial edge, not missing data
 *
 * `docs/23` models "into the workflow" as a transition with no source. A client
 * that treated the absence as an error would refuse the one edge every new task
 * takes; an editor that let a user *author* two of them would break the rule
 * that a workflow has one initial status.
 *
 * # Status deletion always carries a destination
 *
 * `DELETE …/statuses/{sid}?migrate_to={sid}` — a status holding tasks cannot
 * simply vanish, so every task on it moves in the same transaction, attributed
 * to the admin who asked. That is why [`deleteStatus`] has no one-argument form:
 * the destination is not optional and the type says so.
 */
import { request, requestWithVersion } from './http'
import { ApiError } from './problem'

/** A workflow status. `state` is the permanent state it maps onto (`docs/23`). */
export interface WorkflowStatus {
  readonly id: string
  readonly name: string
  readonly state: string
  readonly is_initial: boolean
  readonly position: number
}

/**
 * A legal move.
 *
 * `from` is `null` for the initial edge: `docs/23` models "into the workflow" as
 * a transition with no source, so a client must handle the absence rather than
 * treat it as a data error.
 *
 * `required_permission` is returned as stored so the client can grey out the
 * arrows the actor cannot take, against `GET /permissions/effective`. That is a
 * better refusal than a 403 after the drop.
 */
export interface WorkflowTransition {
  readonly id: string
  readonly from: string | null
  readonly to: string
  readonly required_permission: string | null
  readonly required_fields: readonly string[]
  readonly ignore_dependencies: boolean
}

export interface Workflow {
  readonly id: string
  readonly name: string
  readonly is_default: boolean
  readonly version: number
  /** In `position` order — the order a board draws its columns in. */
  readonly statuses: readonly WorkflowStatus[]
  readonly transitions: readonly WorkflowTransition[]
}

/**
 * The workflow, or `null` when the server does not serve the route yet.
 *
 * Only an **unrouted** 404 becomes `null` — one with no `docs/05` error envelope,
 * which is what axum returns for a path it has no handler for. A 404 that *does*
 * carry `TF-AZN-0008` is the application saying "absent or invisible", and
 * folding that into `null` would render "workflows are not served yet" at a user
 * whose real problem is that they cannot see the project.
 */
export async function readWorkflow(
  workspaceId: string,
  workflowId: string,
  signal?: AbortSignal,
): Promise<Workflow | null> {
  try {
    return await request<Workflow>(`/api/v1/workflows/${workflowId}`, { workspaceId, signal })
  } catch (error) {
    if (isUnrouted(error)) return null
    throw error
  }
}

/**
 * The status to move a card into when it is dropped on a column.
 *
 * A column is a permanent state; a state can hold several statuses. The first by
 * `position` is chosen, which is the workflow author's own ordering — picking by
 * name or by id would be picking arbitrarily and calling it a rule.
 */
export function statusForState(
  workflow: Workflow,
  state: string,
): WorkflowStatus | undefined {
  return workflow.statuses
    .filter((status) => status.state === state)
    .sort((a, b) => a.position - b.position)[0]
}

function isUnrouted(error: unknown): boolean {
  return error instanceof ApiError && error.status === 404 && !error.hasEnvelope
}

/**
 * The workflow with the version its writes need.
 *
 * Distinct from [`readWorkflow`] because the board does not need a version and
 * paying for a second field it ignores is fine — but the *editor* cannot write
 * without one, and a screen that read the body's `version` alone would break the
 * day the shape changed.
 */
export function readWorkflowForEditing(
  workspaceId: string,
  workflowId: string,
  signal?: AbortSignal,
): Promise<{ data: Workflow; version: number | undefined }> {
  return requestWithVersion<Workflow>(`/api/v1/workflows/${workflowId}`, { workspaceId, signal })
}

/** How many tasks sit on each status — what a delete needs to warn about. */
export interface StatusUsage {
  readonly id: string
  readonly name: string
  readonly state: string
  readonly position: number
  readonly is_initial: boolean
  readonly task_count: number
}

export function listStatusUsage(
  workspaceId: string,
  workflowId: string,
  signal?: AbortSignal,
): Promise<{ data: readonly StatusUsage[] }> {
  return request<{ data: readonly StatusUsage[] }>(`/api/v1/workflows/${workflowId}/statuses`, {
    workspaceId,
    signal,
  })
}

/**
 * The five permanent states (`docs/23`).
 *
 * A status maps onto exactly one of them, and the mapping is what every report,
 * filter and "is this done?" question actually reads — the status *name* is a
 * label a workspace chooses. Authoring a status without choosing its state would
 * make the derived column meaningless, so `state` is required here.
 */
export const TASK_STATES = ['BACKLOG', 'PLANNED', 'ACTIVE', 'COMPLETED', 'CANCELED'] as const
export type TaskState = (typeof TASK_STATES)[number]

export function createStatus(
  workspaceId: string,
  workflowId: string,
  version: number,
  status: { name: string; state: TaskState },
): Promise<Workflow> {
  return request<Workflow>(`/api/v1/workflows/${workflowId}/statuses`, {
    method: 'POST',
    workspaceId,
    ifMatch: version,
    body: status,
  })
}

export function updateStatus(
  workspaceId: string,
  workflowId: string,
  statusId: string,
  version: number,
  patch: { name?: string; state?: TaskState; is_initial?: boolean },
): Promise<Workflow> {
  return request<Workflow>(`/api/v1/workflows/${workflowId}/statuses/${statusId}`, {
    method: 'PATCH',
    workspaceId,
    ifMatch: version,
    body: patch,
  })
}

/**
 * Delete a status, moving everything on it to `migrateTo`.
 *
 * Refused with `TF-WFL-0011` when the move is larger than one transaction should
 * carry, and `TF-WFL-0006` when the destination is the status being deleted.
 */
export function deleteStatus(
  workspaceId: string,
  workflowId: string,
  statusId: string,
  version: number,
  migrateTo: string,
): Promise<unknown> {
  return request<unknown>(
    `/api/v1/workflows/${workflowId}/statuses/${statusId}?migrate_to=${migrateTo}`,
    { method: 'DELETE', workspaceId, ifMatch: version },
  )
}

/** The whole order, not a move: a partial list is a workflow with a hole in it. */
export function reorderStatuses(
  workspaceId: string,
  workflowId: string,
  version: number,
  order: readonly string[],
): Promise<Workflow> {
  return request<Workflow>(`/api/v1/workflows/${workflowId}/statuses/order`, {
    method: 'POST',
    workspaceId,
    ifMatch: version,
    body: { order },
  })
}

export function createTransition(
  workspaceId: string,
  workflowId: string,
  version: number,
  edge: {
    from?: string | null
    to: string
    required_permission?: string | null
    required_fields?: readonly string[]
    ignore_dependencies?: boolean
  },
): Promise<Workflow> {
  return request<Workflow>(`/api/v1/workflows/${workflowId}/transitions`, {
    method: 'POST',
    workspaceId,
    ifMatch: version,
    body: edge,
  })
}

export function deleteTransition(
  workspaceId: string,
  workflowId: string,
  transitionId: string,
  version: number,
): Promise<unknown> {
  return request<unknown>(`/api/v1/workflows/${workflowId}/transitions/${transitionId}`, {
    method: 'DELETE',
    workspaceId,
    ifMatch: version,
  })
}
