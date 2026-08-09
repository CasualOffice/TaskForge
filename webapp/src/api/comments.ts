/**
 * `/api/v1/tasks/{id}/comments` and `/api/v1/comments/{id}`.
 *
 * # The failure this module prevents
 *
 * Re-parsing mentions from the comment text. `migrations/0006` resolves mentions
 * **at write time** and stores the ids, because re-resolving `@sam` two years
 * later finds a different Sam or nobody. So [`NewComment.mentions`] is a list of
 * ids the client resolved once, and nothing in this file reads the body looking
 * for an `@`.
 */
import { query, request } from './http'
import type { Paged } from './page'

/** `crates/casual-task-api/src/comments.rs` — `CommentView`. */
export interface Comment {
  readonly id: string
  readonly task_id: string
  /** Threading is one level (`docs/06`): a reply's parent is always top-level. */
  readonly parent_comment_id: string | null
  readonly author_id: string
  readonly body: string
  readonly mentions: readonly string[]
  readonly created_at: string
  readonly edited_at: string | null
  readonly version: number
}

export interface NewComment {
  readonly body: string
  readonly parent_comment_id?: string
  readonly mentions?: readonly string[]
}

/** The thread, oldest first, cursor-paged. */
export function listComments(
  workspaceId: string,
  taskId: string,
  cursor?: string,
  signal?: AbortSignal,
): Promise<Paged<Comment>> {
  return request<Paged<Comment>>(
    `/api/v1/tasks/${taskId}/comments${query({ limit: 50, cursor })}`,
    { workspaceId, signal },
  )
}

export function createComment(
  workspaceId: string,
  taskId: string,
  body: NewComment,
): Promise<Comment> {
  return request<Comment>(`/api/v1/tasks/${taskId}/comments`, {
    method: 'POST',
    workspaceId,
    body,
  })
}

/** Edit your own comment. `If-Match` is required, like every other write. */
export function editComment(
  workspaceId: string,
  commentId: string,
  version: number,
  body: { body: string; mentions?: readonly string[] },
): Promise<Comment> {
  return request<Comment>(`/api/v1/comments/${commentId}`, {
    method: 'PATCH',
    workspaceId,
    ifMatch: version,
    body,
  })
}

/** The schema cap (`migrations/0006`), checked here so an overlong draft is not lost to a 400. */
export const MAX_COMMENT_BYTES = 65_536
