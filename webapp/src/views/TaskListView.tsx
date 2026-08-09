/**
 * The task list: virtualized rows over a keyset-paged feed.
 *
 * # The failure this module prevents
 *
 * Mounting the result set. docs/42 §Rendering strategy: "A 2,000-card board must
 * not mount 2,000 components — and 'our biggest customer has 2,000 cards' is not
 * a hypothetical." The same is true of a list, and it is *easier* to get wrong
 * here because a list of plain rows looks cheap until it is 50,000 of them.
 *
 * So the window is virtualized, the next page is fetched from a cursor when the
 * window approaches the end, and nothing in this file holds an index into a
 * result set that the server has not sent.
 *
 * # Why the rows are a table and not a list of divs
 *
 * Column headers that sort are `<th>` with `aria-sort`, which is what tells a
 * screen-reader user both what a cell means and how the list is ordered. A grid
 * of divs would need four ARIA attributes per row to say the same thing, and the
 * fourth is the one that gets forgotten.
 */
import { useEffect, useMemo, useRef, type ReactElement } from 'react'
import { useVirtualizer } from '@tanstack/react-virtual'

import type { Sort, SortKey, Task, TaskQuery } from '../api/tasks'
import { ErrorNotice } from '../shell/notice'
import { useAppSearch, useOpenTask } from '../shell/navigation'
import { useWorkspaceId } from '../shell/session'
import { useTaskFeed } from '../tasks/feed'
import { formatRelative, isOverdue, priorityLabel, stateLabel } from '../tasks/present'
import { ScopeBar } from './ScopeBar'
import { TaskDrawer } from '../drawer/TaskDrawer'
import { useSortPreference } from './sorting'

const ROW_HEIGHT = 40
/** Fetch the next page while this many rows remain, so scrolling never stalls. */
const PREFETCH_MARGIN = 12

export function TaskListView(): ReactElement {
  const workspaceId = useWorkspaceId()
  const search = useAppSearch()
  const openTask = useOpenTask()
  const [sort, setSort] = useSortPreference()

  const spec = useMemo<TaskQuery>(
    () => ({
      filter: {
        ...(search.project === undefined ? {} : { project: search.project }),
        ...(search.q === undefined ? {} : { q: search.q }),
      },
      sort,
      limit: 100,
    }),
    [search.project, search.q, sort],
  )

  const feed = useTaskFeed(workspaceId, spec)
  const scrollRef = useRef<HTMLDivElement>(null)

  const virtualizer = useVirtualizer({
    count: feed.rows.length,
    getScrollElement: () => scrollRef.current,
    estimateSize: () => ROW_HEIGHT,
    overscan: 10,
  })

  const items = virtualizer.getVirtualItems()
  const lastVisible = items.at(-1)?.index ?? 0

  useEffect(() => {
    if (feed.hasMore && lastVisible >= feed.rows.length - PREFETCH_MARGIN) feed.fetchMore()
  }, [feed, lastVisible])

  return (
    <section className="view" aria-labelledby="list-heading">
      <ScopeBar />
      <div className="view__bar view__bar--sub">
        <h1 id="list-heading" className="view__title">
          Tasks
        </h1>
        <span className="view__count">
          {feed.rows.length}
          {feed.hasMore ? '+' : ''} shown
        </span>
      </div>

      <div className="view__body" ref={scrollRef}>
        {feed.error !== null && feed.error !== undefined ? (
          <div className="list__notice">
            <ErrorNotice error={feed.error} />
          </div>
        ) : null}

        <table className="list" role="table">
          <thead className="list__head">
            <tr>
              <SortableHeader label="Key" column="key" sort={sort} onSort={setSort} />
              <th scope="col" className="list__cell list__cell--title">
                Title
              </th>
              <th scope="col" className="list__cell list__cell--state">
                State
              </th>
              <SortableHeader label="Priority" column="priority" sort={sort} onSort={setSort} />
              <SortableHeader label="Due" column="due_at" sort={sort} onSort={setSort} />
              <SortableHeader label="Updated" column="updated_at" sort={sort} onSort={setSort} />
            </tr>
          </thead>
          <tbody
            className="list__body"
            style={{ height: virtualizer.getTotalSize(), position: 'relative', display: 'block' }}
          >
            {items.map((item) => {
              const task = feed.rows[item.index]
              if (task === undefined) return null
              return (
                <TaskRow
                  key={task.id}
                  task={task}
                  top={item.start}
                  height={item.size}
                  onOpen={openTask}
                />
              )
            })}
          </tbody>
        </table>

        {feed.isPending ? <p className="empty">Loading tasks…</p> : null}
        {!feed.isPending && feed.rows.length === 0 && feed.error == null ? (
          <p className="empty">No tasks match this view.</p>
        ) : null}
        {feed.isFetchingMore ? (
          <p className="empty" role="status">
            Loading more…
          </p>
        ) : null}
      </div>

      {search.task === undefined ? null : <TaskDrawer taskId={search.task} />}
    </section>
  )
}

function TaskRow({
  task,
  top,
  height,
  onOpen,
}: {
  task: Task
  top: number
  height: number
  onOpen: (id: string) => void
}): ReactElement {
  return (
    <tr
      className="list__row"
      style={{ position: 'absolute', top: 0, left: 0, width: '100%', height, transform: `translateY(${top}px)` }}
    >
      <td className="list__cell list__cell--key">
        {/* A button, not a row-level click handler: a clickable `<tr>` is not
            reachable by keyboard and announces nothing. */}
        <button type="button" className="list__open" onClick={() => onOpen(task.id)}>
          <span className="key">{task.key}</span>
        </button>
      </td>
      <td className="list__cell list__cell--title">
        <button type="button" className="list__open list__open--title" onClick={() => onOpen(task.id)}>
          {task.title}
        </button>
      </td>
      <td className="list__cell list__cell--state">
        <span className={`pill pill--${task.state}`}>{stateLabel(task.state)}</span>
      </td>
      <td className="list__cell">
        <span className={`pill pill--${task.priority}`}>{priorityLabel(task.priority)}</span>
      </td>
      <td className={`list__cell${isOverdue(task) ? ' list__cell--overdue' : ''}`}>
        {formatRelative(task.due_at)}
      </td>
      <td className="list__cell">{formatRelative(task.updated_at)}</td>
    </tr>
  )
}

function SortableHeader({
  label,
  column,
  sort,
  onSort,
}: {
  label: string
  column: SortKey
  sort: Sort
  onSort: (next: Sort) => void
}): ReactElement {
  const active = sort.key === column
  return (
    <th
      scope="col"
      className="list__cell"
      aria-sort={active ? (sort.descending ? 'descending' : 'ascending') : 'none'}
    >
      <button
        type="button"
        className="list__sort"
        onClick={() => onSort({ key: column, descending: active ? !sort.descending : true })}
      >
        {label}
        <span aria-hidden="true">{active ? (sort.descending ? ' ↓' : ' ↑') : ''}</span>
      </button>
    </th>
  )
}
