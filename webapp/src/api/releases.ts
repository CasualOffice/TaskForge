/**
 * What went out together (`docs/45` §The two clocks).
 *
 * # The question a release answers and nothing else does
 *
 * A status says what state each task is in. An environment says where each one
 * has reached. Neither says that eleven of them moved *at the same moment*,
 * which is what a deployment conversation is made of and what a rollback needs.
 *
 * # Cutting one is all-or-nothing
 *
 * Unlike `POST /tasks/bulk`, which reports each task's fate separately because
 * those tasks have nothing to do with each other. Here they do: a release that
 * recorded nine of eleven reads as complete, and the two missing ones become
 * invisible in the very surface built to find them. So a refusal means nothing
 * moved, and the caller can say so plainly.
 */
import { request } from './http'

export interface Release {
  readonly id: string
  readonly project_id: string
  readonly name: string
  readonly note: string | null
  readonly created_by: string
  readonly created_at: string
}

export interface ReleasedTask {
  readonly task_id: string
  /** `ONB-14`, so the list reads without a request per row. */
  readonly key: string
  readonly title: string
  readonly promoted_at: string
}

export interface ReleasePage {
  readonly data: readonly Release[]
  readonly page: { readonly next_cursor: string | null; readonly has_more: boolean }
}

export interface CutRelease {
  readonly release: Release
  readonly environment_id: string
  /** What actually moved — never longer than what was sent, and never shorter. */
  readonly task_ids: readonly string[]
}

export function listReleases(
  workspaceId: string,
  projectId: string,
  signal?: AbortSignal,
): Promise<ReleasePage> {
  return request<ReleasePage>(`/api/v1/projects/${projectId}/releases`, { workspaceId, signal })
}

export function readRelease(
  workspaceId: string,
  releaseId: string,
  signal?: AbortSignal,
): Promise<{ release: Release; tasks: readonly ReleasedTask[] }> {
  return request(`/api/v1/releases/${releaseId}`, { workspaceId, signal })
}

/**
 * Record that these tasks went out together, and move each one's environment.
 *
 * Refused with `409` (`TF-PRJ-0015`) when the project already has a release of
 * that name, and `422` when the environment or any task belongs elsewhere — in
 * which case **nothing moved**.
 */
export function cutRelease(
  workspaceId: string,
  projectId: string,
  input: {
    name: string
    note?: string
    environmentId: string
    taskIds: readonly string[]
  },
): Promise<CutRelease> {
  return request<CutRelease>(`/api/v1/projects/${projectId}/releases`, {
    method: 'POST',
    workspaceId,
    body: {
      name: input.name,
      ...(input.note === undefined || input.note === '' ? {} : { note: input.note }),
      environment_id: input.environmentId,
      task_ids: input.taskIds,
    },
  })
}
