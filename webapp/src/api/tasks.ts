/**
 * `/api/v1/tasks` — the read and write surface for a task.
 *
 * # The failure this module prevents
 *
 * A second filter grammar. `docs/27` §Compilation has one AST with two entry
 * points, and the URL form is a closed field set the server refuses to widen —
 * an unknown field is a `400`, never a dropped clause. So [`TaskFilter`] is
 * typed against that exact set (`casual-task-search/src/filter.rs`), and
 * [`SortKey`] against the *smaller* sortable set: a field can be filterable
 * without being sortable, and conflating them is what produces a `TF-QRY-0002`
 * the user never asked for.
 *
 * # Status is not writable here, deliberately
 *
 * [`patchTask`] cannot express a status change: the field is absent from
 * [`TaskPatch`]. `docs/23` makes the transition command the only door, and the
 * server answers `TF-WFL-0001` to anything else — a client type that *could*
 * express it would turn a compile error into a runtime refusal.
 */
import { idempotencyKey, query, request } from './http'
import type { Paged } from './page'

/** `crates/casual-task-api/src/tasks/wire.rs` — `TaskView`. */
export interface Task {
  readonly id: string
  /** `WR-125`. Spans `project.key` and `task.number`, composed server-side (D-051). */
  readonly key: string
  readonly project_id: string
  readonly number: number
  readonly title: string
  readonly description: string | null
  readonly type: TaskType
  readonly priority: Priority
  readonly status_id: string
  /** One of the five permanent states, derived from `status_id` (`docs/23`). */
  readonly state: TaskState
  readonly reporter_id: string
  readonly environment_id: string | null
  readonly milestone_id: string | null
  readonly parent_id: string | null
  readonly start_at: string | null
  readonly due_at: string | null
  /** The lexicographic board rank (ADR-013). */
  readonly position: string
  readonly created_at: string
  readonly created_by: string
  readonly updated_at: string
  readonly updated_by: string | null
  readonly archived_at: string | null
  /** The number that is also the `ETag`. Sent back as `If-Match` on every write. */
  readonly version: number
}

/** `migrations/0001`'s `task_state` enum, in board order. */
export const TASK_STATES = ['BACKLOG', 'PLANNED', 'ACTIVE', 'COMPLETED', 'CANCELED'] as const
export type TaskState = (typeof TASK_STATES)[number]

/** `migrations/0001`'s `task_type` enum. */
export const TASK_TYPES = ['TASK', 'BUG', 'FEATURE', 'INCIDENT', 'REQUEST'] as const
export type TaskType = (typeof TASK_TYPES)[number]

/** `migrations/0001`'s `task_priority` enum, in its declared order. */
export const PRIORITIES = ['NONE', 'LOW', 'MEDIUM', 'HIGH', 'URGENT'] as const
export type Priority = (typeof PRIORITIES)[number]

/**
 * The sortable set — smaller than the filterable one, and closed.
 *
 * `rank` is omitted: it is only meaningful beside a `q` clause, and the server
 * refuses it without one. Offering it in a column header would be offering a
 * `400`.
 */
export const SORT_KEYS = ['updated_at', 'created_at', 'due_at', 'priority', 'position', 'key'] as const
export type SortKey = (typeof SORT_KEYS)[number]

/** `sort=-due_at` — the leading `-` is descending (`docs/27` §URL form). */
export interface Sort {
  readonly key: SortKey
  readonly descending: boolean
}

/**
 * A filter, in the subset of the URL grammar this client emits.
 *
 * Values are written in the grammar's own spelling — `state: 'ACTIVE,PLANNED'`
 * is an `in`, `due_at: '<+7d'` is a `before`, `assignee: '@me'` is a symbol the
 * *server* resolves. They are not pre-resolved here on purpose: `docs/27` says a
 * saved view stores `@me` rather than a user id, and a client that expanded it
 * would produce a filter that is right for its author and wrong for everyone
 * they share it with.
 */
export interface TaskFilter {
  readonly project?: string
  readonly status?: string
  readonly state?: string
  readonly type?: string
  readonly priority?: string
  readonly assignee?: string
  readonly reporter?: string
  readonly tag?: string
  readonly milestone?: string
  readonly environment?: string
  readonly parent?: string
  readonly created_at?: string
  readonly updated_at?: string
  readonly due_at?: string
  readonly key?: string
  readonly title?: string
  readonly q?: string
  readonly is_blocked?: string
  readonly archived?: string
}

export interface TaskQuery {
  readonly filter?: TaskFilter
  readonly sort?: Sort
  readonly limit?: number
  /** Opaque, from a previous page. Never constructed. */
  readonly cursor?: string
}

/** `GET /api/v1/tasks` — one endpoint for lists, boards, My Work, and saved views. */
export function listTasks(
  workspaceId: string,
  spec: TaskQuery,
  signal?: AbortSignal,
): Promise<Paged<Task>> {
  const params: Record<string, string | number | undefined> = { ...spec.filter }
  if (spec.sort !== undefined) {
    params['sort'] = `${spec.sort.descending ? '-' : ''}${spec.sort.key}`
  }
  params['limit'] = spec.limit
  params['cursor'] = spec.cursor
  return request<Paged<Task>>(`/api/v1/tasks${query(params)}`, { workspaceId, signal })
}

export function readTask(
  workspaceId: string,
  taskId: string,
  signal?: AbortSignal,
): Promise<Task> {
  return request<Task>(`/api/v1/tasks/${taskId}`, { workspaceId, signal })
}

/** The fields `PATCH /tasks/{id}` accepts. `null` clears; absent leaves alone. */
export interface TaskPatch {
  readonly title?: string
  readonly description?: string | null
  readonly type?: TaskType
  readonly priority?: Priority
  readonly start_at?: string | null
  readonly due_at?: string | null
}

export function patchTask(
  workspaceId: string,
  taskId: string,
  version: number,
  patch: TaskPatch,
): Promise<Task> {
  return request<Task>(`/api/v1/tasks/${taskId}`, {
    method: 'PATCH',
    workspaceId,
    ifMatch: version,
    body: patch,
  })
}

/** `POST /api/v1/projects/{id}/tasks`. `docs/05`: the create asks for a title. */
export interface NewTask {
  readonly title: string
  readonly description?: string
  readonly type?: TaskType
  readonly priority?: Priority
  readonly parent_id?: string
  readonly due_at?: string
}

export function createTask(
  workspaceId: string,
  projectId: string,
  body: NewTask,
  key = idempotencyKey(),
): Promise<Task> {
  return request<Task>(`/api/v1/projects/${projectId}/tasks`, {
    method: 'POST',
    workspaceId,
    idempotencyKey: key,
    body,
  })
}

/**
 * `POST /api/v1/tasks/{id}/transitions` — the only door to a status change.
 *
 * `fields` satisfies the target transition's `required_fields` and is then
 * discarded server-side: storing it needs custom-field value storage, which is
 * **D-033** and deferred. The default workflow requires no fields, so nothing in
 * the product reaches that gap today.
 */
export function transitionTask(
  workspaceId: string,
  taskId: string,
  version: number,
  body: { to_status_id: string; comment?: string; fields?: Record<string, unknown> },
): Promise<Task> {
  return request<Task>(`/api/v1/tasks/${taskId}/transitions`, {
    method: 'POST',
    workspaceId,
    ifMatch: version,
    body,
  })
}

/**
 * `GET /api/v1/tasks/{id}/assignees` — who is on this task.
 *
 * Ids, not names: a client resolves them through the workspace member directory
 * it already holds, and a second source of display names would be a second thing
 * to keep in step with anonymization (ADR-026).
 *
 * Its own request rather than a field on `TaskView`, because a 200-card board
 * would fetch 200 assignee sets it does not draw — the N+1 `docs/04` §The list
 * problem forbids. The detail surface asks for one.
 */
export function readAssignees(
  workspaceId: string,
  taskId: string,
  signal?: AbortSignal,
): Promise<{ assignees: readonly string[] }> {
  return request<{ assignees: readonly string[] }>(`/api/v1/tasks/${taskId}/assignees`, {
    workspaceId,
    signal,
  })
}

/** `POST /api/v1/tasks/{id}/assignees` — the response is the whole assignee set. */
export function assignTask(
  workspaceId: string,
  taskId: string,
  userId: string,
): Promise<{ assignees: readonly string[] }> {
  return request<{ assignees: readonly string[] }>(`/api/v1/tasks/${taskId}/assignees`, {
    method: 'POST',
    workspaceId,
    body: { user_id: userId },
  })
}

export function unassignTask(
  workspaceId: string,
  taskId: string,
  userId: string,
): Promise<void> {
  return request<void>(`/api/v1/tasks/${taskId}/assignees/${userId}`, {
    method: 'DELETE',
    workspaceId,
  })
}

export function deleteTask(
  workspaceId: string,
  taskId: string,
  version: number,
): Promise<void> {
  return request<void>(`/api/v1/tasks/${taskId}`, {
    method: 'DELETE',
    workspaceId,
    ifMatch: version,
  })
}
