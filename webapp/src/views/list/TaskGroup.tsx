/**
 * One group of a grouped list: its own query, its own cursor, its own count.
 *
 * # Why a group is collapsible and starts open
 *
 * Grouping by state produces five sections, two of which — `COMPLETED` and
 * `CANCELED` — are usually the largest and least interesting. Collapsing is what
 * makes grouping useful rather than merely longer. It starts open because a list
 * that opens fully collapsed hides the work it was asked to show.
 *
 * A collapsed group keeps its query mounted: the header count is the reason
 * someone collapsed it rather than filtering it away, and unmounting the query
 * to save a request would empty the number they were reading.
 *
 * # A group is a `<tbody>`
 *
 * A table may have many bodies, so grouping is a structural feature of the table
 * rather than a second list implementation — the rows, the columns and the sort
 * are literally the same ones. Grouped rows are **not** virtualized: each group
 * is capped at a page with its own "load more", because a virtualizer per group
 * would need N scroll containers inside one scrolling table.
 *
 * # Why an empty group renders at all
 *
 * "Nothing is Blocked" is an answer. A grouped list that dropped its empty
 * sections would make the reader check whether the group exists before trusting
 * that it is empty — and the set of groups is closed and known, so their absence
 * carries no information the presence does not carry better.
 */
import { Button } from '@schnsrw/design-system'
import { useMemo, useState, type ReactElement } from 'react'

import type { Sort, TaskQuery } from '../../api/tasks'
import type { AppSearch } from '../../router'
import { ErrorNotice } from '../../shell/notice'
import { useTaskFeed } from '../../tasks/feed'
import { filterFromSearch } from '../../tasks/query'
import type { Group } from './grouping'
import { TaskRow } from './TaskRow'
import './list.css'

export function TaskGroup({
  workspaceId,
  group,
  search,
  sort,
  onOpen,
}: {
  workspaceId: string
  group: Group
  search: AppSearch
  sort: Sort
  onOpen: (id: string) => void
}): ReactElement {
  const [open, setOpen] = useState(true)

  const spec = useMemo<TaskQuery>(
    () => ({
      // The group's own constraint wins over the toolbar's: a group IS that
      // value, and letting the filter override it would make every group show
      // the same rows.
      filter: { ...filterFromSearch(search), ...group.scope },
      sort,
      limit: 100,
    }),
    [search, group.scope, sort],
  )

  const feed = useTaskFeed(workspaceId, spec)

  const note = (text: string): ReactElement => (
    <tr className="group__note">
      <td colSpan={6}>{text}</td>
    </tr>
  )

  return (
    <tbody className="group">
      <tr className="group__head">
        <th scope="colgroup" colSpan={6}>
          <button
            type="button"
            className="group__toggle"
            aria-expanded={open}
            onClick={() => setOpen(!open)}
          >
            <span className="group__chevron" aria-hidden="true">
              {open ? '▾' : '▸'}
            </span>
            <span>{group.title}</span>
            <span className="group__count">
              {feed.rows.length}
              {feed.hasMore ? '+' : ''}
            </span>
          </button>
        </th>
      </tr>

      {open ? (
        <>
          {feed.error != null ? (
            <tr>
              <td colSpan={6}>
                <ErrorNotice error={feed.error} />
              </td>
            </tr>
          ) : null}
          {feed.isPending ? note('Loading…') : null}
          {!feed.isPending && feed.rows.length === 0 && feed.error == null
            ? note('Nothing here.')
            : null}

          {feed.rows.map((task) => (
            <TaskRow key={task.id} task={task} onOpen={onOpen} />
          ))}

          {feed.hasMore ? (
            <tr>
              <td colSpan={6}>
                <Button
                  variant="subtle"
                  className="group__more"
                  onClick={feed.fetchMore}
                  disabled={feed.isFetchingMore}
                >
                  {feed.isFetchingMore ? 'Loading…' : `Load more ${group.title.toLowerCase()}`}
                </Button>
              </td>
            </tr>
          ) : null}
        </>
      ) : null}
    </tbody>
  )
}
