/**
 * What happened to this task, newest first.
 *
 * # Why it is behind a disclosure and the comments are not
 *
 * A conversation is what people come to a task for; its history is what they
 * come to when something looks wrong. `design/DESIGN-FOUNDATION.md` §1.8 —
 * scrolling is a cost — so the surface spends its vertical budget on the thread
 * and keeps the trail one press away. Closed, it costs a line; open, it is a
 * bounded list with a "load more" rather than an infinite scroll that never
 * settles.
 *
 * # It is not fetched until it is opened
 *
 * `enabled` follows the disclosure. Every task view would otherwise pay for a
 * history most readers never open — and on a board where the peek opens on
 * hover, that is one request per card looked at.
 *
 * # `task.history.read`, checked before asking
 *
 * The endpoint refuses without it (`docs/04`). Asking anyway would put a 403 in
 * the console on every task a reader opens, and the disclosure would open onto
 * an error rather than not being offered.
 */
import { FLUSH } from '../shell/controls'
import { Button } from '@schnsrw/design-system'
import { useState, type ReactElement } from 'react'
import { useInfiniteQuery } from '@tanstack/react-query'

import { describe, readActivity, type ActivityEntry } from '../api/activity'
import { keys } from '../api/keys'
import { PERMISSIONS } from '../api/permissions'
import { nextCursor } from '../api/page'
import { ErrorNotice } from '../shell/notice'
import type { Authority } from '../shell/permissions'
import { useWorkspaceId } from '../shell/session'
import { formatRelative } from '../tasks/present'

export function Activity({
  taskId,
  authority,
}: {
  taskId: string
  authority: Authority
}): ReactElement | null {
  const workspaceId = useWorkspaceId()
  const [open, setOpen] = useState(false)

  const history = useInfiniteQuery({
    queryKey: keys.activity(workspaceId, taskId),
    queryFn: ({ pageParam, signal }) =>
      readActivity(workspaceId, taskId, pageParam as string | undefined, signal),
    initialPageParam: undefined as string | undefined,
    getNextPageParam: (last) => nextCursor(last.page),
    enabled: open && workspaceId !== '',
  })

  if (!authority.can(PERMISSIONS.taskHistoryRead)) return null

  const entries = (history.data?.pages ?? []).flatMap((page) => page.data)

  return (
    <section className="act" aria-labelledby="activity-heading">
      <h2 id="activity-heading" className="narr__heading">
        <Button variant="subtle" style={FLUSH} aria-expanded={open} onClick={() => setOpen(!open)}>
          Activity
        </Button>
      </h2>

      {!open ? null : (
        <>
          {history.error ? <ErrorNotice error={history.error} /> : null}
          {history.isPending ? <p className="act__empty">Loading…</p> : null}
          {!history.isPending && entries.length === 0 ? (
            <p className="act__empty">Nothing has happened to this task yet.</p>
          ) : null}

          <ol className="act__list">
            {entries.map((entry) => (
              <Entry key={entry.id} entry={entry} />
            ))}
          </ol>

          {history.hasNextPage ? (
            <Button
              variant="subtle"
              disabled={history.isFetchingNextPage}
              onClick={() => void history.fetchNextPage()}
            >
              {history.isFetchingNextPage ? 'Loading…' : 'Older'}
            </Button>
          ) : null}
        </>
      )}
    </section>
  )
}

function Entry({ entry }: { entry: ActivityEntry }): ReactElement {
  return (
    <li className="act__row">
      <span className="act__who">
        {/* A null actor is the system — a sweeper, a migration, the dispatcher.
            "Someone" would be a lie and a blank would look like a bug. */}
        {entry.actor_name ?? (entry.actor_id === null ? 'TaskForge' : 'A former member')}
      </span>{' '}
      <span className="act__what">{describe(entry)}</span>{' '}
      <time className="act__when" dateTime={entry.occurred_at} title={entry.occurred_at}>
        {formatRelative(entry.occurred_at)}
      </time>
    </li>
  )
}
