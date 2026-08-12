/**
 * My Work — what is mine, by three readings of "mine".
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
 * # An empty lens offers the next one
 *
 * "Assigned to me" is genuinely empty on a fresh workspace, because nothing is
 * assigned yet. An empty screen that says only "nothing here" is indistinguishable
 * from a broken query, so the empty state names the *other* readings of "mine"
 * and links to them. That is not a guess about intent — the user already told us
 * they want their own work; they have not told us which sense of "own".
 *
 * # The gap is a footnote, not a banner
 *
 * Watched tasks cannot be queried (`watcher` is not in the closed field set and
 * no endpoint exposes watchers). That is worth saying and it is not worth the
 * top of the screen — a notice above the content is a notice that outranks the
 * content, which is exactly the mistake the board's old "choose a project"
 * banner made.
 */
import { Button } from '@schnsrw/design-system'
import { useMemo, type ReactElement } from 'react'

import type { TaskQuery } from '../api/tasks'
import { useAppSearch, useOpenTask, useUpdateSearch } from '../shell/navigation'
import { PageHeader } from '../shell/PageHeader'
import { WorkToolbar } from './filters/WorkToolbar'
import { ErrorNotice, GapNotice } from '../shell/notice'
import { TaskLink } from '../task/TaskLink'
import { useWorkspaceId } from '../shell/session'
import { useTaskFeed } from '../tasks/feed'
import { formatRelative, isOverdue } from '../tasks/present'
import { PriorityBadge, TypeBadge } from '../tasks/TaskCard'

type Lens = 'assigned' | 'reported' | 'overdue'

const LENSES: readonly [
  { id: Lens; label: string; empty: string },
  ...{ id: Lens; label: string; empty: string }[],
] = [
  { id: 'assigned', label: 'Assigned to me', empty: 'Nothing is assigned to you.' },
  { id: 'reported', label: 'Reported by me', empty: 'You have not reported anything.' },
  { id: 'overdue', label: 'Overdue', empty: 'Nothing of yours is overdue.' },
]

/** Soonest deadline first: the ordering the question "what should I do next" implies. */
const SORT = { key: 'due_at', descending: false } as const

export function MyWorkView(): ReactElement {
  const workspaceId = useWorkspaceId()
  const search = useAppSearch()
  const update = useUpdateSearch()
  const openTask = useOpenTask()

  // The lens is read back out of the URL rather than held in state, so a link to
  // "my overdue work" opens on that lens and not on whatever the recipient last
  // clicked. `assigned` is the default because it is what "My Work" means to
  // someone who has not chosen — arriving with no parameters is not the same as
  // asking for what you reported.
  const lens: Lens =
    search.reporter === '@me'
      ? 'reported'
      : search.due !== undefined && search.due !== ''
        ? 'overdue'
        : 'assigned'

  function choose(next: Lens): void {
    if (next === 'assigned') update({ assignee: '@me', reporter: undefined, due: undefined })
    else if (next === 'overdue') update({ assignee: '@me', reporter: undefined, due: '<@today' })
    else update({ assignee: undefined, reporter: '@me', due: undefined })
  }

  const spec = useMemo<TaskQuery>(() => {
    // Finished work is not "my work". `!` is the grammar's not-in.
    const base: Record<string, string> = { state: '!COMPLETED,CANCELED' }
    if (lens === 'reported') base['reporter'] = '@me'
    else base['assignee'] = '@me'
    // `@today` is resolved in the *actor's* offset by the server, which is the
    // whole reason the symbol exists — a client sending an absolute instant
    // would compute midnight against the browser's clock instead.
    if (lens === 'overdue') base['due_at'] = '<@today'
    return { filter: base, sort: SORT, limit: 100 }
  }, [lens])

  const feed = useTaskFeed(workspaceId, spec)
  // `LENSES` is typed as non-empty, so the fallback is a real value rather than
  // `possibly undefined` — the list is a constant and an empty one would be a
  // bug, not a state to handle at every use.
  const active = LENSES.find((option) => option.id === lens) ?? LENSES[0]

  return (
    <section className="view" aria-labelledby="page-title">
      {/* `PageHeader`, like every other route. This drew its own bar with its
          own `<h1>`, so My work was the one page in the product with no
          breadcrumb — you could not see which workspace you were in, and the
          title sat at a different size from every page you reached it from. */}
      <PageHeader title="My work" count={`${feed.rows.length} shown`} />

      <WorkToolbar>
        {/* One choice of three, so they are pressed buttons rather than links:
            `aria-pressed` says which reading of "mine" is in force, which a
            group of plain buttons cannot.

            They were styled with `.button`, `.button--primary` and
            `.button--quiet` — three class names that exist in **no**
            stylesheet, so the most important control on the page rendered as
            bare browser buttons. The design system draws them now. */}
        <div className="lenses" role="group" aria-label="Which of my tasks">
          {LENSES.map((option) => (
            <Button
              key={option.id}
              size="sm"
              variant={lens === option.id ? 'primary' : 'subtle'}
              aria-pressed={lens === option.id}
              onClick={() => choose(option.id)}
            >
              {option.label}
            </Button>
          ))}
        </div>
      </WorkToolbar>

      <div className="view__body mywork">
        {feed.error != null ? <ErrorNotice error={feed.error} /> : null}
        {feed.isPending ? <p className="empty">Loading…</p> : null}

        {!feed.isPending && feed.rows.length === 0 && feed.error == null ? (
          <div className="empty">
            <p>{active.empty}</p>
            <div className="empty__actions">
              {LENSES.filter((option) => option.id !== lens).map((option) => (
                <Button variant="secondary" key={option.id} onClick={() => choose(option.id)}>
                  {option.label}
                </Button>
              ))}
            </div>
          </div>
        ) : null}

        <ul className="mywork__list">
          {feed.rows.map((task) => (
            <li key={task.id}>
              <TaskLink taskId={task.id} className="mywork__row" onPeek={openTask}>
                <TypeBadge type={task.type} />
                <span className="key">{task.key}</span>
                <span className="mywork__title">{task.title}</span>
                {task.priority === 'NONE' ? <span /> : <PriorityBadge priority={task.priority} />}
                <span className={isOverdue(task) ? 'mywork__due mywork__due--late' : 'mywork__due'}>
                  {task.due_at === null ? (
                    ''
                  ) : (
                    <time dateTime={task.due_at}>
                      {isOverdue(task) ? 'overdue ' : 'due '}
                      {formatRelative(task.due_at)}
                    </time>
                  )}
                </span>
              </TaskLink>
            </li>
          ))}
        </ul>

        {feed.hasMore ? (
          <Button variant="subtle" onClick={feed.fetchMore} disabled={feed.isFetchingMore}>
            {feed.isFetchingMore ? 'Loading…' : 'Load more'}
          </Button>
        ) : null}

        <footer className="mywork__footnote">
          {/* `watcher` is not in the filter grammar's closed field set and no
              endpoint exposes watchers — which is why this says so rather than
              quietly returning a smaller answer than the heading promises. */}
          <GapNotice what="Tasks you only watch are not included yet." />
        </footer>
      </div>

    </section>
  )
}
