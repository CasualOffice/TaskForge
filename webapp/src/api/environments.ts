/**
 * `/api/v1/projects/{id}/environments` — the pipeline a project deploys along.
 *
 * Ordered by `position`, which is the deployment order — dev, qa, staging,
 * production — and therefore the order the environment board draws its columns
 * in. Sorting by name would put production second.
 */
import { request } from './http'

export interface Environment {
  readonly id: string
  readonly project_id: string
  readonly name: string
  readonly position: number
}

export function listEnvironments(
  workspaceId: string,
  projectId: string,
  signal?: AbortSignal,
): Promise<{ data: readonly Environment[] }> {
  return request<{ data: readonly Environment[] }>(`/api/v1/projects/${projectId}/environments`, {
    workspaceId,
    signal,
  })
}

/**
 * Put a project's environments in the given order.
 *
 * The whole order, not one move: moving one environment changes the position of
 * every environment it passed, and a per-item call is a read-modify-write two
 * people can hold at once. The caller states what the pipeline *is*.
 *
 * Refused with `400` unless the ids are exactly this project's environments,
 * each once — a partial order would leave one unplaced and nothing on screen
 * would say so.
 */
export function reorderEnvironments(
  workspaceId: string,
  projectId: string,
  environmentIds: readonly string[],
): Promise<{ data: readonly Environment[] }> {
  return request<{ data: readonly Environment[] }>(
    `/api/v1/projects/${projectId}/environments/order`,
    { method: 'PUT', workspaceId, body: { environment_ids: environmentIds } },
  )
}

/** `POST /api/v1/projects/{id}/environments`. Refused `409` on a duplicate name. */
export function createEnvironment(
  workspaceId: string,
  projectId: string,
  name: string,
): Promise<Environment> {
  return request<Environment>(`/api/v1/projects/${projectId}/environments`, {
    method: 'POST',
    workspaceId,
    body: { name },
  })
}

/** `PATCH /api/v1/environments/{id}`. */
export function renameEnvironment(
  workspaceId: string,
  environmentId: string,
  name: string,
): Promise<Environment> {
  return request<Environment>(`/api/v1/environments/${environmentId}`, {
    method: 'PATCH',
    workspaceId,
    body: { name },
  })
}

/**
 * `DELETE /api/v1/environments/{id}?migrate_to=…`
 *
 * The target is required and there is no default. Tasks carrying an environment
 * that vanishes are tasks whose history stops explaining them, so the caller
 * says where they go — another environment, or the literal `none` to clear the
 * field. Untagging four thousand tasks is a decision, not a fallback.
 */
export function deleteEnvironment(
  workspaceId: string,
  environmentId: string,
  migrateTo: string,
): Promise<unknown> {
  return request<unknown>(
    `/api/v1/environments/${environmentId}?migrate_to=${encodeURIComponent(migrateTo)}`,
    { method: 'DELETE', workspaceId },
  )
}
