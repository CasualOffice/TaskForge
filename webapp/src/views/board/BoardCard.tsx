/**
 * A board card: the shared task card, plus the ability to pick it up.
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
 * Opening the task is a *second* button, inside `TaskCard`, because a control
 * that both drags and activates on Enter has to guess which the user meant.
 *
 * Everything about how a task *looks* lives in `TaskCard` and not here — see
 * that module for why the board must not draw its own.
 */
import type { CSSProperties, ReactElement } from 'react'
import { useDraggable } from '@dnd-kit/core'

import type { Task } from '../../api/tasks'
import { TaskCard } from '../../tasks/TaskCard'

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
    // The task itself travels with the drag so the overlay — rendered by the
    // context, outside every column — can draw it. The board never holds the
    // task list; each column queries its own.
    data: { statusId: task.status_id, state: task.state, task },
  })

  // Written out rather than pulled from `@dnd-kit/utilities`: docs/42 commits to
  // an exact dependency set, and BUNDLE-FLOOR.md measured that set. Adding a
  // package for one string template would make the floor a measurement of
  // something the app no longer is.
  // No transform. The card stays where it was, dimmed to mark the gap it came
  // from, and `DragOverlay` draws the moving copy at the top of the page.
  //
  // Translating it here is what put the card *behind* everything: it remained a
  // child of `.column__body`, which scrolls, so it was clipped by its own column
  // and painted under every column to its right. No `z-index` reaches out of an
  // ancestor's `overflow` clip.
  const style: CSSProperties = {
    opacity: isDragging ? 0.4 : 1,
  }
  void transform

  return (
    <div ref={setNodeRef} style={style} className="card__host">
      <TaskCard
        task={task}
        onOpen={onOpen}
        handle={
          draggable ? (
            <button
              type="button"
              className="card__grip"
              aria-label={`Move ${task.key}`}
              {...attributes}
              {...listeners}
            >
              <span aria-hidden="true">⠿</span>
            </button>
          ) : null
        }
      />
    </div>
  )
}
