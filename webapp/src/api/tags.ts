/**
 * `/api/v1/tags` — the tag vocabulary.
 *
 * # Why a tag is created here and never by typing one on a task
 *
 * Authoring the vocabulary is `tag.manage`; applying an existing tag is
 * `task.update`. A picker that created a tag from whatever was typed would make
 * every typo a permanent term in a shared vocabulary, and would do it under the
 * wrong permission. So the task surface applies tags by id, and new terms are
 * authored here.
 *
 * # Scope
 *
 * A tag with no `project_id` belongs to the workspace and every project may use
 * it. One with a `project_id` is that project's alone, and `task::usable_tag`
 * refuses it elsewhere with a `422` — which is why a picker attached to a task
 * must pass `project_id` rather than listing everything.
 */
import { query, request } from './http'

export interface Tag {
  readonly id: string
  /** `null` for a workspace-wide tag — the common case. */
  readonly project_id: string | null
  readonly name: string
  /** A presentation hint only; every surface renders the name too. */
  readonly color: string | null
}

/**
 * The vocabulary. Whole, not paged: it is configuration, and a cursor over it
 * would make every client paginate to draw a dropdown.
 */
export function listTags(
  workspaceId: string,
  projectId?: string,
  signal?: AbortSignal,
): Promise<{ data: readonly Tag[] }> {
  return request<{ data: readonly Tag[] }>(`/api/v1/tags${query({ project_id: projectId })}`, {
    workspaceId,
    signal,
  })
}

/** Needs `tag.manage`. Refused with `TF-PRJ-0011` if the name is taken at that scope. */
export function createTag(
  workspaceId: string,
  tag: { name: string; project_id?: string | undefined; color?: string | undefined },
): Promise<Tag> {
  return request<Tag>('/api/v1/tags', { method: 'POST', workspaceId, body: tag })
}
