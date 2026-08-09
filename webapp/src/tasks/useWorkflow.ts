/**
 * The workflow behind a project, and what to do when there isn't one yet.
 *
 * # The failure this module prevents
 *
 * Four components each deciding, differently, what to do about a missing
 * `GET /api/v1/workflows/{id}`. The endpoint is specified in `docs/05` and not
 * yet served (see `api/workflow.ts`, tracked as **D-061**), which means every
 * status-changing control in the app has the same three-way answer to give:
 * available, unavailable-because-unbuilt, or refused. Deciding that once, here,
 * is what stops the board disabling a drag while the drawer happily sends a
 * transition that cannot succeed.
 */
import { useQuery } from '@tanstack/react-query'

import { keys } from '../api/keys'
import { readProject } from '../api/projects'
import { readWorkflow, statusForState, type Workflow } from '../api/workflow'
import { useAuthority } from '../shell/permissions'
import { useWorkspaceId } from '../shell/session'

export interface WorkflowState {
  readonly workflow: Workflow | undefined
  readonly loading: boolean
  /**
   * Why status cannot be changed right now, or `undefined` when it can.
   *
   * A sentence rather than a boolean: the control that disables itself is the
   * control that has to say why, and a `disabled` prop with no explanation is
   * how an interface becomes unexplainable.
   */
  readonly unavailable: string | undefined
  /** The status a card dropped on a column should move into. */
  readonly statusFor: (state: string) => string | undefined
  /**
   * The move from one status into a column, if the workflow has one.
   *
   * Returns the edge's own verdict as well as its target, because `docs/23`
   * step 5 gives a transition a `required_permission` on top of
   * `task.transition`: an actor who may move tasks in general may still not make
   * one particular move. A board that only checked the general permission would
   * accept the drop, send it, and roll the card back on a `TF-WFL-0003`.
   */
  readonly moveInto: (fromStatusId: string, toState: string) => Move | undefined
}

/** A legal edge into a column, and whether this actor may take it. */
export interface Move {
  readonly toStatusId: string
  readonly toState: string
  readonly permitted: boolean
  /** The permission the edge demands, when it demands one. */
  readonly needed: string | null
}

const NOT_SERVED =
  'Status changes need the workflow’s statuses, and this server does not serve ' +
  'GET /api/v1/workflows/{id} yet (docs/05 specifies it; C-007 has not shipped it).'

const NO_PROJECT = 'Choose a project — a workflow belongs to one, not to the whole workspace.'

export function useProjectWorkflow(projectId: string | undefined): WorkflowState {
  const workspaceId = useWorkspaceId()
  const authority = useAuthority(projectId)

  const project = useQuery({
    queryKey: [...keys.projects(workspaceId), projectId ?? ''],
    queryFn: ({ signal }) => readProject(workspaceId, projectId ?? '', signal),
    enabled: workspaceId !== '' && projectId !== undefined,
    staleTime: 60_000,
  })

  const workflowId = project.data?.workflow_id

  const workflow = useQuery({
    queryKey: keys.workflow(workspaceId, workflowId ?? ''),
    queryFn: ({ signal }) => readWorkflow(workspaceId, workflowId ?? '', signal),
    enabled: workspaceId !== '' && workflowId !== undefined,
    staleTime: 5 * 60_000,
    // The route being absent is not a transient failure; retrying it four times
    // per mount would spend four round trips learning the same thing.
    retry: false,
  })

  const loaded = workflow.data ?? undefined
  const loading = project.isPending || workflow.isPending

  let unavailable: string | undefined
  if (projectId === undefined) unavailable = NO_PROJECT
  else if (!loading && loaded === undefined) unavailable = NOT_SERVED

  return {
    workflow: loaded,
    loading,
    unavailable,
    statusFor: (state) => (loaded === undefined ? undefined : statusForState(loaded, state)?.id),
    moveInto: (fromStatusId, toState) => {
      if (loaded === undefined) return undefined
      // Every status in the target column, not just the first: the workflow may
      // have an edge into the second one and none into the first, and refusing
      // the drop because of the order statuses happen to be in would refuse a
      // move the workflow permits.
      const candidates = loaded.statuses
        .filter((status) => status.state === toState)
        .sort((a, b) => a.position - b.position)
      for (const candidate of candidates) {
        const edge = loaded.transitions.find(
          (transition) => transition.from === fromStatusId && transition.to === candidate.id,
        )
        if (edge === undefined) continue
        const needed = edge.required_permission
        return {
          toStatusId: candidate.id,
          toState: candidate.state,
          permitted: needed === null || authority.can(needed),
          needed,
        }
      }
      return undefined
    },
  }
}
