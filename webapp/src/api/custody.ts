/**
 * The chain of custody (`docs/45`): who holds a task, where it has reached, and
 * how it fared.
 *
 * # Two clocks
 *
 * A task's *status* is what state the work is in; its *environment* is where it
 * has reached. They advance independently, which is why "resolved, on qa,
 * verified there, not yet on staging" is an ordinary sentence and no status
 * column can say it.
 *
 * # Why one read for three lists
 *
 * They are one panel and always rendered together. Three endpoints would render
 * it in three stages, and the panel's whole job is to answer "what happened to
 * this" at a glance.
 */
import { request } from './http'

export interface Transfer {
  readonly id: string
  /** `null` for the first hand-off out of triage — a transfer from nobody. */
  readonly from_team_id: string | null
  readonly to_team_id: string
  readonly moved_by: string
  readonly moved_at: string
  readonly note: string | null
}

export interface Promotion {
  readonly id: string
  readonly environment_id: string
  readonly release_id: string | null
  readonly promoted_by: string
  readonly promoted_at: string
}

export type Verdict = 'PASS' | 'FAIL'

export interface Verification {
  readonly id: string
  readonly environment_id: string
  readonly verdict: Verdict
  readonly verified_by: string
  readonly verified_at: string
  readonly note: string | null
}

export interface Custody {
  /** `null` means untriaged — not missing data, but the triage queue. */
  readonly team_id: string | null
  readonly environment_id: string | null
  readonly transfers: readonly Transfer[]
  readonly promotions: readonly Promotion[]
  readonly verifications: readonly Verification[]
}

export function readCustody(
  workspaceId: string,
  taskId: string,
  signal?: AbortSignal,
): Promise<Custody> {
  return request<Custody>(`/api/v1/tasks/${taskId}/custody`, { workspaceId, signal })
}

/**
 * Hand the task to another team.
 *
 * **This clears the assignees**, so the task lands in the receiving team's
 * queue. Any control that calls it must say so before it is pressed — a hand-off
 * that silently unassigned someone would be the worst kind of surprise.
 *
 * Refused with `409` when the task already belongs to that team, and `422`
 * (`TF-VAL-0007`) when the team is not on the task's project.
 */
export function transferTeam(
  workspaceId: string,
  taskId: string,
  teamId: string,
  note?: string,
): Promise<Transfer> {
  return request<Transfer>(`/api/v1/tasks/${taskId}/team`, {
    method: 'PUT',
    workspaceId,
    body: { team_id: teamId, ...(note === undefined || note === '' ? {} : { note }) },
  })
}

/**
 * Record that it reached an environment.
 *
 * Deliberately not idempotent: a second promotion to the same environment is a
 * redeploy, which is a real event.
 */
export function promote(
  workspaceId: string,
  taskId: string,
  environmentId: string,
): Promise<Promotion> {
  return request<Promotion>(`/api/v1/tasks/${taskId}/promotions`, {
    method: 'POST',
    workspaceId,
    body: { environment_id: environmentId },
  })
}

/**
 * Record a verdict against the environment it was tested on.
 *
 * The environment defaults to the one the task is on, which is the ordinary case
 * — QA tests what was pushed. Recording a verdict does **not** move the task;
 * what follows is a transition the caller makes next, and keeping them separate
 * is what lets "failed twice on qa" survive later status changes.
 */
export function verify(
  workspaceId: string,
  taskId: string,
  verdict: Verdict,
  note?: string,
  environmentId?: string,
): Promise<Verification> {
  return request<Verification>(`/api/v1/tasks/${taskId}/verifications`, {
    method: 'POST',
    workspaceId,
    body: {
      verdict,
      ...(note === undefined || note === '' ? {} : { note }),
      ...(environmentId === undefined ? {} : { environment_id: environmentId }),
    },
  })
}
