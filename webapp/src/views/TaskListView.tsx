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
import { useEffect, useMemo, useRef, useState, type ReactElement } from 'react'
import { useVirtualizer } from '@tanstack/react-virtual'

import { PRIORITIES, TASK_TYPES, type Sort, type SortKey, type TaskQuery } from '../api/tasks'
import { ErrorNotice } from '../shell/notice'
import { useAppSearch, useOpenTask } from '../shell/navigation'
import { useWorkspaceId } from '../shell/session'
import { useLiveUpdates } from '../live/useLiveUpdates'
import { useTaskFeed } from '../tasks/feed'
import { filterFromSearch } from '../tasks/query'
import { priorityLabel, typeLabel } from '../tasks/present'
import { FilterHeader } from './list/FilterHeader'
import { listMembers, directory } from '../api/workspaces'
import { keys } from '../api/keys'
import { useQuery } from '@tanstack/react-query'
import { useProjectWorkflow } from '../tasks/useWorkflow'
import { CreateTask } from './CreateTask'
import { PageHeader } from '../shell/PageHeader'
import { WorkToolbar } from './filters/WorkToolbar'
import { TaskGroup } from './list/TaskGroup'
import { TaskRow } from './list/TaskRow'
import { groupsFor, type GroupKey } from './list/grouping'
import { useSortPreference } from './sorting'
import { useNarrow } from '../shell/viewport'

/**
 * A first guess, replaced by measurement.
 *
 * Two values because the two compositions are two layouts: the desktop row is
 * one table line, the narrow row is a four-line stacked summary. This is only
 * the *estimate* the virtualizer starts from — every rendered row reports its
 * real height through `measureElement`, which is what keeps a wrapped title
 * from being clipped. A single constant here was audit item 3.
 */
const ROW_HEIGHT = 40
const ROW_HEIGHT_NARROW = 104
/** Fetch the next page while this many rows remain, so scrolling never stalls. */
const PREFETCH_MARGIN = 12

export function TaskListView(): ReactElement {
  const workspaceId = useWorkspaceId()
  const search = useAppSearch()
  const openTask = useOpenTask()
  const [sort, setSort] = useSortPreference()
  const [group, setGroup] = useState<GroupKey | undefined>(undefined)
  const { workflow } = useProjectWorkflow(search.project)

  // The vocabularies the column filters offer. Statuses belong to the scoped
  // project's workflow; at workspace scope there is no single set to offer, so
  // that column filters on nothing rather than on a list that is wrong for most
  // of the rows in front of the reader.
  const statusOptions = (workflow?.statuses ?? []).map((status) => ({
    value: status.id,
    label: status.name,
  }))
  const statusName = (id: string): string | undefined =>
    (workflow?.statuses ?? []).find((status) => status.id === id)?.name

  const members = useQuery({
    queryKey: keys.members(workspaceId),
    queryFn: ({ signal }) => listMembers(workspaceId, signal),
    enabled: workspaceId !== '',
    staleTime: 60_000,
  })
  const nameOf = directory(members.data?.data ?? [])
  // Empty unless grouping is on. Each group runs its own keyset-paged query —
  // see `list/grouping.ts` for why the page is never grouped in the browser.
  const groups = group === undefined ? [] : groupsFor(group, workflow)

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
  const narrow = useNarrow()

  const virtualizer = useVirtualizer({
    count: feed.rows.length,
    getScrollElement: () => scrollRef.current,
    estimateSize: () => (narrow ? ROW_HEIGHT_NARROW : ROW_HEIGHT),
    overscan: 10,
  })

  const items = virtualizer.getVirtualItems()
  const lastVisible = items.at(-1)?.index ?? 0

  useEffect(() => {
    if (feed.hasMore && lastVisible >= feed.rows.length - PREFETCH_MARGIN) feed.fetchMore()
  }, [feed, lastVisible])

  return (
    <section className="view" aria-labelledby="page-title">
      <PageHeader
        title="List"
        count={`${feed.rows.length}${feed.hasMore ? '+' : ''} shown`}
        actions={<CreateTask projectId={search.project} />}
      />
      <WorkToolbar
        sort={sort}
        onSort={setSort}
        group={group}
        onGroup={setGroup}
        workflow={workflow}
        onColumns={['status', 'priority', 'type']}
      />

      <div className="view__body" ref={scrollRef}>
        {feed.error !== null && feed.error !== undefined ? (
          <div className="list__notice">
            <ErrorNotice error={feed.error} />
          </div>
        ) : null}

        <table className="list" role="table">
          <thead className="list__head">
            <tr>
              <FilterHeader
                label="Type"
                field="type"
                options={TASK_TYPES.map((t) => ({ value: t, label: typeLabel(t) }))}
                className="list__cell--type"
              />
              <SortableHeader label="Key" column="key" sort={sort} onSort={setSort} />
              <th scope="col" className="list__cell list__cell--title">
                Title
              </th>
              {/* Filters live on the column they narrow, which is where every
                  application of this kind puts them. A toolbar of dropdowns
                  makes a reader map "Status" in one place onto a column in
                  another, and the mapping is the work. */}
              <FilterHeader label="Status" field="status" options={statusOptions} />
              {/* Display only. The assignee filter stays in the toolbar because
                  it carries three meanings a checkbox list cannot — anyone, me,
                  and *unassigned*, which the grammar spells as a
                  present-and-empty value. A column filter here would silently
                  drop the one of those people use most. */}
              <th scope="col" className="list__cell">
                Assignee
              </th>
              <FilterHeader
                label="Priority"
                field="priority"
                options={PRIORITIES.map((p) => ({ value: p, label: priorityLabel(p) }))}
                sortColumn="priority"
                sort={sort}
                onSort={setSort}
              />
              <SortableHeader label="Due" column="due_at" sort={sort} onSort={setSort} />
              <SortableHeader label="Updated" column="updated_at" sort={sort} onSort={setSort} />
            </tr>
          </thead>
          {groups.length > 0 ? (
            // One `<tbody>` per group, each with its own query and cursor. The
            // columns, the sort and the rows are the table's own — grouping is a
            // structural feature of it, not a second list beside it.
            groups.map((entry) => (
              <TaskGroup
                key={entry.id}
                workspaceId={workspaceId}
                group={entry}
                statusName={statusName}
                nameOf={nameOf}
                search={search}
                sort={sort}
                onOpen={openTask}
              />
            ))
          ) : (
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
                    statusName={statusName}
                    nameOf={nameOf}
                    onOpen={openTask}
                    measureRef={virtualizer.measureElement}
                    index={item.index}
                    style={{
                      position: 'absolute',
                      top: 0,
                      left: 0,
                      width: '100%',
                      // No `height`. The row reports its own through
                      // `measureElement`; pinning it here is what clipped the
                      // stacked row to one line of a four-line summary.
                      transform: `translateY(${item.start}px)`,
                    }}
                  />
                )
              })}
            </tbody>
          )}
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

    </section>
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
