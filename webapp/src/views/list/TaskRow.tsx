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
import { formatRelative, isOverdue } from '../../tasks/present'
import { PriorityBadge, TypeBadge } from '../../tasks/TaskCard'

export function TaskRow({
  task,
  onOpen,
  style,
}: {
  task: Task
  onOpen: (id: string) => void
  /** Absolute positioning, supplied only by the virtualized ungrouped list. */
  style?: React.CSSProperties
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
