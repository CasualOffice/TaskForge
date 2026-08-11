/**
 * One row of the task list.
 *
 * # Why this is a `<tr>` and not a styled `<div>`
 *
 * The list is a table: columns have headers, the headers sort, and `aria-sort`
 * on a `<th>` is what tells a screen-reader user both what a cell means and how
 * the list is currently ordered. A grid of divs needs four ARIA attributes per
 * row to say the same thing, and the fourth is the one that gets forgotten.
 *
 * Being a `<tr>` is also what lets the grouped list reuse it: a table may have
 * many `<tbody>` elements, so a group is a body with a header row rather than a
 * second implementation of a row.
 *
 * # Two links, not a clickable row
 *
 * A `<tr>` with an `onClick` is not reachable by keyboard and announces nothing.
 * The identifier and the title are anchors to the task's own route, so ⌘-click
 * and middle-click open a tab; a plain click opens the peek instead and keeps
 * the reader's place. `task/TaskLink.tsx` owns that rule for every surface.
 */
import type { ReactElement } from 'react'

import type { Task } from '../../api/tasks'
import { TaskLink } from '../../task/TaskLink'
import { formatRelative, isOverdue, stateLabel } from '../../tasks/present'
import { PriorityBadge, TypeBadge } from '../../tasks/TaskCard'

export function TaskRow({
  task,
  onOpen,
  style,
  statusName,
  nameOf,
}: {
  task: Task
  onOpen: (id: string) => void
  /** Absolute positioning, supplied only by the virtualized ungrouped list. */
  style?: React.CSSProperties
  /**
   * The project's own name for a status, when the view is scoped to one.
   *
   * Absent at workspace scope, where statuses belong to several workflows and
   * two projects may both have a "Done" that is not the same status. The row
   * falls back to the permanent state, which `docs/23` derives from the status
   * and which therefore cannot contradict it — less specific, never wrong.
   */
  statusName?: (id: string) => string | undefined
  nameOf?: (id: string) => string
}): ReactElement {
  return (
    <tr className="list__row" style={style}>
      <td className="list__cell list__cell--type">
        <TypeBadge type={task.type} />
      </td>
      <td className="list__cell list__cell--key">
        <TaskLink taskId={task.id} className="list__open" onPeek={onOpen}>
          <span className="key">{task.key}</span>
        </TaskLink>
      </td>
      <td className="list__cell list__cell--title">
        <TaskLink taskId={task.id} className="list__open list__open--title" onPeek={onOpen}>
          {task.title}
        </TaskLink>
      </td>
      <td className="list__cell">
        {/* Status, in words. It was absent, and it is the field people scan a
            list for — "what state is this in" is the question the list exists
            to answer for many rows at once. The name comes from the project's
            workflow; the permanent state behind it is what the colour says. */}
        <span className={`statuspill statuspill--${task.state.toLowerCase()}`}>
          {statusName?.(task.status_id) ?? stateLabel(task.state)}
        </span>
      </td>
      <td className="list__cell list__cell--who">
        {/* Who is on it. Also absent, and the second thing anyone scans for —
            a list that cannot say whose work this is cannot answer "what is
            mine" without opening every row. */}
        {/* From the page's own payload — the list resolves every assignee in
            one query, so this costs no request. */}
        {(task.assignees ?? []).length === 0 ? (
          <span className="list__nobody">—</span>
        ) : (
          (task.assignees ?? []).map((id) => nameOf?.(id) ?? id).join(', ')
        )}
      </td>
      <td className="list__cell">
        {/* `NONE` renders as nothing — see `tasks/TaskCard.tsx`. A pill reading
            "None" on most rows occupies the position where a signal would be, so
            the eye stops checking it and misses the URGENT. */}
        {task.priority === 'NONE' ? null : <PriorityBadge priority={task.priority} />}
      </td>
      <td className={`list__cell${isOverdue(task) ? ' list__cell--overdue' : ''}`}>
        {task.due_at === null ? (
          '—'
        ) : (
          <time dateTime={task.due_at}>{formatRelative(task.due_at)}</time>
        )}
      </td>
      <td className="list__cell">
        <time dateTime={task.updated_at}>{formatRelative(task.updated_at)}</time>
      </td>
    </tr>
  )
}
