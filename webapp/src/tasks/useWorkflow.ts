/**
 * The workflow behind a project, and what to do when there isn't one yet.
 *
 * # The failure this module prevents
 *
 * Four components each deciding, differently, what to do about a missing
 * `GET /api/v1/workflows/{id}`. The endpoint is specified in `docs/05` and not
 * yet served (see `api/workflow.ts`, tracked as **D-059**), which means every
 * status-changing control in the app has the same three-way answer to give:
 * available, unavailable-because-unbuilt, or refused. Deciding that once, here,
 * is what stops the board disabling a drag while the drawer happily sends a
 * transition that cannot succeed.
 */
import { useQuery } from '@tanstack/react-query'

import { keys } from '../api/keys'
import { readProject } from '../api/projects'
import { readWorkflow, statusForState, type Workflow } from '../api/workflow'
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
}

const NOT_SERVED =
  'Status changes need the workflow’s statuses, and this server does not serve ' +
  'GET /api/v1/workflows/{id} yet (docs/05 specifies it; C-007 has not shipped it).'

const NO_PROJECT = 'Choose a project — a workflow belongs to one, not to the whole workspace.'

export function useProjectWorkflow(projectId: string | undefined): WorkflowState {
  const workspaceId = useWorkspaceId()

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
  }
}
