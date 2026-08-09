/**
 * My Work — what is assigned to me, and what I reported.
 *
 * # Why `@me` is sent and not a user id
 *
 * `docs/27` §Symbolic values: "A saved view stores `@me`, not a user id... a view
 * that hardcoded a user id would be shareable but wrong." The server resolves
 * the symbol against the authenticated actor, so this URL is correct for
 * whoever opens it — including a colleague someone pastes it to. Expanding it
 * client-side would produce a link that quietly shows the sender's work to the
 * recipient.
 *
 * # The gap: watched tasks
 *
 * The brief for this view is "assigned *and watched*". `watcher` is not in the
 * filter grammar's closed field set (`casual-task-search/src/filter.rs`) and no
 * endpoint exposes watchers, so there is nothing to query. `reporter=@me` is
 * offered instead — the other half of "mine" that the grammar *can* express —
 * and the missing half is stated rather than quietly dropped.
 */
import { useMemo, useState, type ReactElement } from 'react'

import type { TaskQuery } from '../api/tasks'
import { TaskDrawer } from '../drawer/TaskDrawer'
import { useAppSearch, useOpenTask } from '../shell/navigation'
import { ErrorNotice, GapNotice } from '../shell/notice'
import { useWorkspaceId } from '../shell/session'
import { useTaskFeed } from '../tasks/feed'
import { formatRelative, isOverdue, priorityLabel, stateLabel } from '../tasks/present'

type Lens = 'assigned' | 'reported' | 'overdue'

const LENSES: readonly { id: Lens; label: string }[] = [
  { id: 'assigned', label: 'Assigned to me' },
  { id: 'reported', label: 'Reported by me' },
  { id: 'overdue', label: 'Overdue' },
]

export function MyWorkView(): ReactElement {
  const workspaceId = useWorkspaceId()
  const search = useAppSearch()
  const openTask = useOpenTask()
  const [lens, setLens] = useState<Lens>('assigned')

  const spec = useMemo<TaskQuery>(() => {
    const base = {
      // Finished work is not "my work". `!` is the grammar's not-in.
      state: '!COMPLETED,CANCELED',
      ...(search.project === undefined ? {} : { project: search.project }),
    }
    if (lens === 'reported') return { filter: { ...base, reporter: '@me' }, sort: SORT, limit: 100 }
    if (lens === 'overdue') {
      // `@today` is resolved in the *actor's* offset by the server, which is the
      // whole reason the symbol exists — a client sending an absolute instant
      // would compute midnight against the browser's clock instead.
      return { filter: { ...base, assignee: '@me', due_at: '<@today' }, sort: SORT, limit: 100 }
    }
    return { filter: { ...base, assignee: '@me' }, sort: SORT, limit: 100 }
  }, [lens, search.project])

  const feed = useTaskFeed(workspaceId, spec)

  return (
    <section className="view" aria-labelledby="mywork-heading">
      <div className="view__bar">
        <h1 id="mywork-heading" className="view__title">
          My Work
        </h1>
        <div className="lenses" role="group" aria-label="Which of my tasks">
          {LENSES.map((option) => (
            <button
              key={option.id}
              type="button"
              className={`button${lens === option.id ? ' button--primary' : ' button--quiet'}`}
              aria-pressed={lens === option.id}
              onClick={() => setLens(option.id)}
            >
              {option.label}
            </button>
          ))}
        </div>
        <span className="shell__spacer" />
        <span className="view__count">{feed.rows.length}</span>
      </div>

      <div className="view__body mywork">
        <GapNotice what="Watched tasks are not included." tracker="C-008">
          <span>
            <code>watcher</code> is not in the filter grammar’s closed field set and no endpoint
            exposes watchers, so there is nothing to query yet.
          </span>
        </GapNotice>

        {feed.error != null ? <ErrorNotice error={feed.error} /> : null}
        {feed.isPending ? <p className="empty">Loading…</p> : null}
        {!feed.isPending && feed.rows.length === 0 && feed.error == null ? (
          <p className="empty">Nothing here. That is allowed to be good news.</p>
        ) : null}

        <ul className="mywork__list">
          {feed.rows.map((task) => (
            <li key={task.id}>
              <button type="button" className="mywork__row" onClick={() => openTask(task.id)}>
                <span className="key">{task.key}</span>
                <span className="mywork__title">{task.title}</span>
                <span className={`pill pill--${task.state}`}>{stateLabel(task.state)}</span>
                <span className={`pill pill--${task.priority}`}>{priorityLabel(task.priority)}</span>
                <span className={isOverdue(task) ? 'mywork__due mywork__due--late' : 'mywork__due'}>
                  {task.due_at === null ? '' : `due ${formatRelative(task.due_at)}`}
                </span>
              </button>
            </li>
          ))}
        </ul>

        {feed.hasMore ? (
          <button
            type="button"
            className="button button--quiet"
            onClick={feed.fetchMore}
            disabled={feed.isFetchingMore}
          >
            {feed.isFetchingMore ? 'Loading…' : 'Load more'}
          </button>
        ) : null}
      </div>

      {search.task === undefined ? null : <TaskDrawer taskId={search.task} />}
    </section>
  )
}

/** Soonest deadline first: the ordering the question "what should I do next" implies. */
const SORT = { key: 'due_at', descending: false } as const
