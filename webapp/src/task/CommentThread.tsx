/**
 * The comment thread.
 *
 * # The failure this module prevents
 *
 * A comment that disappears because the request failed. docs/42 §Optimistic
 * mutation: "A failed comment keeps its text in the draft cache. This is the
 * difference between a blip and a betrayal." So the draft is cleared in
 * `onSuccess` and nowhere else — not on submit, not on unmount — and the error
 * renders *beside* the text the user still has.
 *
 * # Threading is one level, and the UI says so
 *
 * `docs/06` caps replies at one level and the server rejects a reply to a reply.
 * Only top-level comments therefore offer a Reply control; offering it on a
 * reply would be offering a refusal.
 */
import { useState, type FormEvent, type ReactElement } from 'react'
import { useInfiniteQuery, useMutation, useQueryClient } from '@tanstack/react-query'

import { keys } from '../api/keys'
import { MAX_COMMENT_BYTES, createComment, listComments, type Comment } from '../api/comments'
import { nextCursor } from '../api/page'
import { PERMISSIONS } from '../api/permissions'
import { directory, listMembers } from '../api/workspaces'
import { useAnnounce } from '../shell/announce'
import { ErrorNotice } from '../shell/notice'
import type { Authority } from '../shell/permissions'
import { useWorkspaceId } from '../shell/session'
import { formatRelative } from '../tasks/present'
import { useQuery } from '@tanstack/react-query'

export function CommentThread({
  taskId,
  authority,
}: {
  taskId: string
  authority: Authority
}): ReactElement {
  const workspaceId = useWorkspaceId()
  const mayComment = authority.can(PERMISSIONS.taskComment)
  const announce = useAnnounce()
  const client = useQueryClient()
  const [draft, setDraft] = useState('')
  const [replyTo, setReplyTo] = useState<string | undefined>(undefined)

  const thread = useInfiniteQuery({
    queryKey: keys.comments(workspaceId, taskId),
    queryFn: ({ pageParam, signal }) => listComments(workspaceId, taskId, pageParam, signal),
    initialPageParam: undefined as string | undefined,
    getNextPageParam: (last) => nextCursor(last.page),
    enabled: workspaceId !== '',
  })

  const members = useQuery({
    queryKey: keys.members(workspaceId),
    queryFn: ({ signal }) => listMembers(workspaceId, signal),
    enabled: workspaceId !== '',
    staleTime: 5 * 60_000,
  })
  const nameOf = directory(members.data?.data ?? [])

  const post = useMutation({
    mutationFn: () =>
      createComment(workspaceId, taskId, {
        body: draft.trim(),
        ...(replyTo === undefined ? {} : { parent_comment_id: replyTo }),
      }),
    onSuccess: () => {
      // Only here. A `finally` would clear the box on failure too, which is the
      // exact behaviour this component exists to avoid.
      setDraft('')
      setReplyTo(undefined)
      announce('Comment posted')
      void client.invalidateQueries({ queryKey: keys.comments(workspaceId, taskId) })
    },
  })

  const comments = (thread.data?.pages ?? []).flatMap((page) => page.data)
  const roots = comments.filter((comment) => comment.parent_comment_id === null)
  const repliesTo = (id: string): Comment[] =>
    comments.filter((comment) => comment.parent_comment_id === id)

  const tooLong = new TextEncoder().encode(draft).length > MAX_COMMENT_BYTES

  function submit(event: FormEvent): void {
    event.preventDefault()
    if (draft.trim() !== '' && !tooLong) post.mutate()
  }

  return (
    <div className="thread-panel">
      {thread.isPending ? <p className="field__hint">Loading thread…</p> : null}
      {thread.error != null ? <ErrorNotice error={thread.error} /> : null}
      {!thread.isPending && roots.length === 0 ? (
        <p className="field__hint">No comments yet.</p>
      ) : null}

      <ol className="thread">
        {roots.map((comment) => (
          <li key={comment.id} className="thread__item">
            <CommentBody comment={comment} nameOf={nameOf} />
            {mayComment ? (
              <button
                type="button"
                className="button button--quiet thread__reply"
                onClick={() => setReplyTo(comment.id)}
              >
                Reply
              </button>
            ) : null}
            {repliesTo(comment.id).length === 0 ? null : (
              <ol className="thread thread--replies">
                {repliesTo(comment.id).map((reply) => (
                  <li key={reply.id} className="thread__item">
                    <CommentBody comment={reply} nameOf={nameOf} />
                  </li>
                ))}
              </ol>
            )}
          </li>
        ))}
      </ol>

      {thread.hasNextPage ? (
        <button
          type="button"
          className="button button--quiet"
          onClick={() => void thread.fetchNextPage()}
          disabled={thread.isFetchingNextPage}
        >
          Load earlier comments
        </button>
      ) : null}

      {mayComment ? (
      <form className="thread__composer" onSubmit={submit}>
        <label className="field__label" htmlFor="comment-draft">
          {replyTo === undefined ? 'Add a comment' : 'Reply'}
        </label>
        <textarea
          id="comment-draft"
          className="textarea"
          rows={3}
          value={draft}
          onChange={(event) => setDraft(event.target.value)}
        />
        {tooLong ? (
          <span className="field__hint">A comment is limited to 64 KiB.</span>
        ) : null}
        <div className="field__actions">
          <button
            type="submit"
            className="button button--primary"
            disabled={draft.trim() === '' || tooLong || post.isPending}
          >
            {post.isPending ? 'Posting…' : 'Comment'}
          </button>
          {replyTo === undefined ? null : (
            <button type="button" className="button button--quiet" onClick={() => setReplyTo(undefined)}>
              Cancel reply
            </button>
          )}
        </div>
        {post.isError ? <ErrorNotice error={post.error} /> : null}
      </form>
      ) : (
        <p className="field__hint">You do not have permission to comment on this task.</p>
      )}
    </div>
  )
}

function CommentBody({
  comment,
  nameOf,
}: {
  comment: Comment
  nameOf: (id: string) => string
}): ReactElement {
  return (
    <article className="comment">
      <header className="comment__head">
        <strong>{nameOf(comment.author_id)}</strong>
        <time dateTime={comment.created_at} className="comment__when">
          {formatRelative(comment.created_at)}
        </time>
        {comment.edited_at === null ? null : <span className="comment__when">· edited</span>}
      </header>
      {/* Rendered as text, not markup. The body is untrusted user content and
          the rich-text editor is lazy-loaded on *edit* only (docs/42 §Stack). */}
      <p className="comment__body">{comment.body}</p>
    </article>
  )
}
