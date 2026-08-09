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
import { useLiveUpdates } from '../live/useLiveUpdates'
import { useTaskFeed } from '../tasks/feed'
import { formatRelative, isOverdue } from '../tasks/present'
import { PriorityBadge, TypeBadge } from '../tasks/TaskCard'
import { filterFromSearch } from '../tasks/query'
import { CreateTask } from './CreateTask'
import { WorkToolbar } from './filters/WorkToolbar'
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

  // Every filter the toolbar set, translated once in `tasks/query.ts` so the
  // list, the board and My Work cannot disagree about what the address means.
  const spec = useMemo<TaskQuery>(
    () => ({ filter: filterFromSearch(search), sort, limit: 100 }),
    [search, sort],
  )

  const feed = useTaskFeed(workspaceId, spec)
  // Only when a project is scoped: the stream has no all-projects form, and the
  // server refusing one is the point rather than a limitation to work around.
  useLiveUpdates(workspaceId, search.project)
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
      <WorkToolbar sort={sort} onSort={setSort}>
        <CreateTask projectId={search.project} />
      </WorkToolbar>
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
              <th scope="col" className="list__cell list__cell--type">
                Type
              </th>
              <SortableHeader label="Key" column="key" sort={sort} onSort={setSort} />
              <th scope="col" className="list__cell list__cell--title">
                Title
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
          // design/LAYOUT §10: empty states are operational. This is that
          // document's own example sentence, verbatim.
          <p className="empty">No tasks match this view. Change the filters or create a task.</p>
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
      <td className="list__cell list__cell--type">
        <TypeBadge type={task.type} />
      </td>
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
      <td className="list__cell">
        {/* `NONE` renders as nothing — see `tasks/TaskCard.tsx`. A pill reading
            "None" on most rows occupies the position where a signal would be,
            so the eye stops checking it and misses the URGENT. */}
        {task.priority === 'NONE' ? null : <PriorityBadge priority={task.priority} />}
      </td>
      <td className={`list__cell${isOverdue(task) ? ' list__cell--overdue' : ''}`}>
        {task.due_at === null ? (
          '—'
        ) : (
          <time dateTime={task.due_at}>{formatRelative(task.due_at)}</time>
        )}
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
