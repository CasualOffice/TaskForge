/**
 * One card on the board.
 *
 * # The failure this module prevents
 *
 * A card that only a mouse can move. docs/42 §Accessibility: "Keyboard-operable
 * everything, including drag and drop (dnd-kit's keyboard sensor is a
 * requirement, not a nice-to-have)." A `<div>` with pointer listeners fails that
 * silently — it looks finished and excludes everyone who does not use a mouse.
 *
 * So the drag handle is a real `<button>`: it is in the tab order for free, it
 * announces itself, and dnd-kit's keyboard sensor drives it with the arrow keys.
 * Opening the task is a *second* button, because a control that both drags and
 * activates on Enter has to guess which the user meant.
 */
import type { CSSProperties, ReactElement } from 'react'
import { useDraggable } from '@dnd-kit/core'

import type { Task } from '../../api/tasks'
import { formatRelative, isOverdue, priorityLabel } from '../../tasks/present'

export function BoardCard({
  task,
  onOpen,
  draggable,
}: {
  task: Task
  onOpen: (id: string) => void
  draggable: boolean
}): ReactElement {
  const { attributes, listeners, setNodeRef, transform, isDragging } = useDraggable({
    id: task.id,
    disabled: !draggable,
    data: { state: task.state },
  })

  // Written out rather than pulled from `@dnd-kit/utilities`: docs/42 commits to
  // an exact dependency set, and BUNDLE-FLOOR.md measured that set. Adding a
  // package for one string template would make the floor a measurement of
  // something the app no longer is.
  const style: CSSProperties = {
    transform: transform === null ? undefined : `translate3d(${transform.x}px, ${transform.y}px, 0)`,
    opacity: isDragging ? 0.5 : 1,
  }

  return (
    <article className="card" ref={setNodeRef} style={style}>
      <div className="card__head">
        <button type="button" className="card__key" onClick={() => onOpen(task.id)}>
          <span className="key">{task.key}</span>
        </button>
        {draggable ? (
          <button
            type="button"
            className="card__grip"
            aria-label={`Move ${task.key}`}
            {...attributes}
            {...listeners}
          >
            <span aria-hidden="true">⠿</span>
          </button>
        ) : null}
      </div>

      <button type="button" className="card__title" onClick={() => onOpen(task.id)}>
        {task.title}
      </button>

      <div className="card__foot">
        <span className={`pill pill--${task.priority}`}>{priorityLabel(task.priority)}</span>
        {task.due_at === null ? null : (
          <span className={isOverdue(task) ? 'card__due card__due--late' : 'card__due'}>
            due {formatRelative(task.due_at)}
          </span>
        )}
      </div>
    </article>
  )
}
