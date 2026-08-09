/**
 * `GET /api/v1/workflows/{id}` — the board's columns, and the ids a transition
 * needs.
 *
 * # This endpoint is specified and not yet built
 *
 * `docs/05` §Other resources lists `GET /api/v1/workflows/{id}`. The server does
 * not serve it: `crates/casual-task-api/src/server.rs` registers no
 * `/workflows` route, because C-007 (default workflow + transitions) is still
 * `Building` and shipped only the write half — the state machine and the
 * transition command.
 *
 * That single gap is what stops a board from moving a card. `POST
 * /tasks/{id}/transitions` takes a `to_status_id`, and there is no other way for
 * a browser to learn one: `TaskView` carries the *current* `status_id` and the
 * derived `state`, never the workflow's status set.
 *
 * # Why the client calls it anyway
 *
 * This module is written against the documented contract, so the day the route
 * is registered the board starts working with **no client change**. Until then
 * [`readWorkflow`] resolves to `null` on a 404, the board renders its columns
 * from the five permanent states (`docs/23`), and every cross-column drag is
 * refused *in the UI* with the reason named — rather than sent, rejected with a
 * code the user cannot act on, and rolled back.
 *
 * Recorded as **D-059** in `docs/14-EXECUTION-TRACKER.md`. It is a missing
 * endpoint, not an open design question, so nothing here invents a shape:
 * the fields below are `casual_task_persistence::workflow::load`'s two rows.
 */
import { request } from './http'
import { ApiError } from './problem'

/** A workflow status. `state` is the permanent state it maps onto (`docs/23`). */
export interface WorkflowStatus {
  readonly id: string
  readonly name: string
  readonly state: string
  readonly is_initial: boolean
  readonly position: number
}

/** A legal move. `from` is null for the initial-status edge. */
export interface WorkflowTransition {
  readonly id: string
  readonly from: string | null
  readonly to: string
  readonly required_permission: string | null
  readonly required_fields: readonly string[]
}

export interface Workflow {
  readonly id: string
  readonly name: string
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
