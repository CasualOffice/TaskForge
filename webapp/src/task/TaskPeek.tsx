/**
 * `TaskPeek` — a quick look at a task without leaving the board or the list.
 *
 * # What this is not
 *
 * It is not a smaller detail page. `design/LAYOUT-AND-INTERACTION-GUIDELINES.md`
 * §4: "Use the drawer only for a **peek** … showing identity, state, assignee
 * and description, with an obvious route to the full view. It is not a smaller
 * version of the detail page; it is a different, deliberately partial thing."
 *
 * The distinction matters because the previous drawer tried to be the whole
 * detail surface in 480 px and produced the failure the specification now names:
 * a reader had to scroll to learn the assignee, and the conversation, the
 * relations and the dates were all below the fold.
 *
 * So the peek shows **only what fits**: who, what state, when it is due, and the
 * opening of the description. Everything else is one click away on a surface
 * built to hold it. It does not scroll, by construction — if a field would not
 * fit, it does not belong here.
 *
 * # It shares its parts with the full route
 *
 * §9 of the foundation requires one component set across the two, so a field
 * cannot exist in one and be forgotten in the other. `StatusControl` and the
 * assignee row here are the same components the detail route renders.
 */
import { useCallback, useRef, type ReactElement } from 'react'
import { Link } from '@tanstack/react-router'
import { useQuery } from '@tanstack/react-query'

import { keys } from '../api/keys'
import { readTask, type Task } from '../api/tasks'
import { directory, listMembers } from '../api/workspaces'
import { useFocusTrap } from '../shell/focusTrap'
import { useOpenTask } from '../shell/navigation'
import { ErrorNotice } from '../shell/notice'
import { useAuthority } from '../shell/permissions'
import { useWorkspaceId } from '../shell/session'
import { formatRelative, isOverdue } from '../tasks/present'
import { PriorityBadge, TypeBadge } from '../tasks/TaskCard'
import { StatusControl } from './StatusControl'

/** How much of a description a peek shows before deferring to the full view. */
const SNIPPET = 320

export function TaskPeek({ taskId }: { taskId: string }): ReactElement {
  const workspaceId = useWorkspaceId()
  const openTask = useOpenTask()
  const panel = useRef<HTMLDivElement>(null)

  const close = useCallback(() => openTask(undefined), [openTask])
  useFocusTrap(panel, close)

  const task = useQuery({
    queryKey: keys.task(workspaceId, taskId),
    queryFn: ({ signal }) => readTask(workspaceId, taskId, signal),
    enabled: workspaceId !== '',
  })

  const members = useQuery({
    queryKey: keys.members(workspaceId),
    queryFn: ({ signal }) => listMembers(workspaceId, signal),
    enabled: workspaceId !== '',
    staleTime: 5 * 60_000,
  })
  const nameOf = directory(members.data?.data ?? [])

  const authority = useAuthority(task.data?.project_id)
  const current = task.data
  /** Present the day `TaskView` carries the set; see `TaskMeta`. */
  const assignees =
    current === undefined ? undefined : (current as Task & { assignees?: readonly string[] }).assignees
  const description = current?.description ?? ''
  const clipped = description.length > SNIPPET

  return (
    <div className="peek">
      {/* The scrim closes on click but is `aria-hidden`: it is a target, not a
          control, and announcing it would put a nameless button in the reading
          order. Escape is the keyboard equivalent, wired by the focus trap. */}
      <div className="peek__scrim" onClick={close} aria-hidden="true" />

      <div
        className="peek__panel"
        ref={panel}
        role="dialog"
        aria-modal="true"
        aria-labelledby="peek-title"
        tabIndex={-1}
      >
        <header className="peek__head">
          <span className="key">{current?.key ?? '…'}</span>
          <span className="shell__spacer" />
          <button type="button" className="button button--quiet" onClick={close}>
            Close
          </button>
        </header>

        {task.isPending ? <p className="empty">Loading task…</p> : null}
        {task.error != null ? (
          <div className="peek__body">
            <ErrorNotice error={task.error} />
          </div>
        ) : null}

        {current === undefined ? null : (
          <div className="peek__body">
            <h2 id="peek-title" className="peek__title">
              {current.title}
            </h2>

            {/* The four signals a peek exists to answer, on one line, above
                everything else. §8: where am I, what is here, what needs
                attention — before what can I do next. */}
            <div className="peek__signals">
              <StatusControl task={current} authority={authority} />
              <TypeBadge type={current.type} />
              {current.priority === 'NONE' ? null : <PriorityBadge priority={current.priority} />}
              {current.due_at === null ? null : (
                <span className={`peek__due${isOverdue(current) ? ' peek__due--late' : ''}`}>
                  {isOverdue(current) ? 'overdue ' : 'due '}
                  <time dateTime={current.due_at}>{formatRelative(current.due_at)}</time>
                </span>
              )}
            </div>

            {/* §4 names assignee among the four things a peek must answer.
                It is the field the old drawer buried below the fold, so it is
                the last one that may be left off this list. */}
            <p className="peek__who">
              <span className="peek__wholabel">Assignees</span>
              <span className={assignees === undefined || assignees.length === 0 ? 'meta2__unset' : ''}>
                {assignees === undefined
                  ? 'Not shown yet'
                  : assignees.length === 0
                    ? 'Nobody'
                    : assignees.map(nameOf).join(', ')}
              </span>
            </p>

            {description === '' ? (
              <p className="narr__none">No description.</p>
            ) : (
              <p className="peek__desc">
                {clipped ? `${description.slice(0, SNIPPET).trimEnd()}…` : description}
              </p>
            )}

            {/* The obvious route §4 asks for. A `Link`, so middle-click and
                ⌘-click open a real tab — the reason this is a route at all. */}
            <Link
              to="/tasks/$taskId"
              params={{ taskId: current.id }}
              className="button button--primary peek__open"
            >
              Open full view
            </Link>
          </div>
        )}
      </div>
    </div>
  )
}
