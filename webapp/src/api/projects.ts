/**
 * `/api/v1/projects` — the container every task belongs to.
 *
 * Changes when `docs/05` §Projects does. Kept apart from `tasks.ts` because the
 * project contract has moved twice for reasons that had nothing to do with tasks
 * (ADR-007 froze `key`; visibility gained `TEAM`).
 */
import { idempotencyKey, query, request } from './http'
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

/**
 * Create a project.
 *
 * `key` is required and **permanent** (ADR-007): it prefixes every task key
 * this project will ever mint, and those keys end up in commit messages, chat
 * and other people's tickets. The server refuses anything but 2–10 characters
 * starting with an uppercase letter, and refuses a duplicate with a `409` — so
 * the form says so before the attempt rather than translating a refusal after.
 */
export function createProject(
  workspaceId: string,
  project: {
    key: string
    name: string
    description?: string | undefined
    visibility?: ProjectVisibility | undefined
  },
): Promise<Project> {
  return request<Project>('/api/v1/projects', {
    method: 'POST',
    workspaceId,
    idempotencyKey: idempotencyKey(),
    body: project,
  })
}

/**
 * The visibility values `docs/22`'s enum permits, in widening order.
 *
 * Ordered deliberately: a picker that listed them alphabetically would put
 * `PRIVATE` between `TEAM` and `WORKSPACE` and make "who can see this" read as
 * an arbitrary list rather than as a scale.
 */
export const VISIBILITIES = ['PRIVATE', 'TEAM', 'WORKSPACE'] as const
export type ProjectVisibility = (typeof VISIBILITIES)[number]

/**
 * Rename a project, or change what it says and who can see it.
 *
 * `key` is deliberately absent from the type: the server answers `422`
 * (`TF-PRJ-…`) to any attempt to change it, and a field that exists in the
 * client type is a field someone will eventually send.
 */
export function updateProject(
  workspaceId: string,
  projectId: string,
  version: number,
  patch: {
    name?: string | undefined
    description?: string | null | undefined
    visibility?: ProjectVisibility | undefined
  },
): Promise<Project> {
  return request<Project>(`/api/v1/projects/${projectId}`, {
    method: 'PATCH',
    workspaceId,
    ifMatch: version,
    body: patch,
  })
}
