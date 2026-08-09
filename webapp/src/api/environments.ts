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
  return request<{ data: readonly Environment[] }>(
    `/api/v1/projects/${projectId}/environments`,
    { workspaceId, signal },
  )
}
