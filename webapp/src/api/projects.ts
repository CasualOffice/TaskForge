/**
 * `/api/v1/projects` — the container every task belongs to.
 *
 * Changes when `docs/05` §Projects does. Kept apart from `tasks.ts` because the
 * project contract has moved twice for reasons that had nothing to do with tasks
 * (ADR-007 froze `key`; visibility gained `TEAM`).
 */
import { query, request } from './http'
import type { Paged } from './page'

/** `crates/casual-task-api/src/projects.rs` — `ProjectView`. */
export interface Project {
  readonly id: string
  /** Immutable after creation (ADR-007) — it appears in every task key. */
  readonly key: string
  readonly name: string
  readonly description: string | null
  readonly visibility: string
  readonly team_id: string | null
  /** The workflow whose statuses the board's columns come from. */
  readonly workflow_id: string
  readonly created_at: string
  readonly created_by: string
  readonly updated_at: string
  readonly updated_by: string | null
  readonly archived_at: string | null
  readonly version: number
}

export function listProjects(workspaceId: string, signal?: AbortSignal): Promise<Paged<Project>> {
  return request<Paged<Project>>(`/api/v1/projects${query({ limit: 100 })}`, {
    workspaceId,
    signal,
  })
}

export function readProject(
  workspaceId: string,
  projectId: string,
  signal?: AbortSignal,
): Promise<Project> {
  return request<Project>(`/api/v1/projects/${projectId}`, { workspaceId, signal })
}
