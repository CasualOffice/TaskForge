/**
 * The task drawer: detail over the view that opened it.
 *
 * # The failure this module prevents
 *
 * Losing the user's place. docs/42: detail opens "in a drawer over the board,
 * preserving scroll position and context". Routing to a full page instead
 * unmounts the board, refetches it, and returns the user to the top of a list
 * they had scrolled — which is why people stop opening tasks.
 *
 * # What this file does and does not own
 *
 * It owns the shell: the scrim, the dialog semantics, the focus contract, and
 * which panels appear. Editing lives in `TaskFields`, the thread in
 * `CommentThread`, the status change in `TransitionControl`. They are separate
 * because they change for different reasons — a field rule, a comment rule, and
 * the workflow are three different documents (`docs/05`, `docs/06`, `docs/23`).
 */
import { useCallback, useRef, type ReactElement } from 'react'
import { useQuery } from '@tanstack/react-query'

import { keys } from '../api/keys'
import { readTask } from '../api/tasks'
import { useFocusTrap } from '../shell/focusTrap'
import { useOpenTask } from '../shell/navigation'
import { ErrorNotice, GapNotice } from '../shell/notice'
import { useWorkspaceId } from '../shell/session'
import { formatDateTime } from '../tasks/present'
import { Assignees } from './Assignees'
import { CommentThread } from './CommentThread'
import { TaskFields } from './TaskFields'
import { TransitionControl } from './TransitionControl'

export function TaskDrawer({ taskId }: { taskId: string }): ReactElement {
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

  return (
    <div className="drawer">
      {/* The scrim closes on click but is `aria-hidden`: it is a target, not a
          control, and announcing it would put a nameless button in the reading
          order. Escape is the keyboard equivalent, wired by the focus trap. */}
      <div className="drawer__scrim" onClick={close} aria-hidden="true" />

      <div
        className="drawer__panel"
        ref={panel}
        role="dialog"
        aria-modal="true"
        aria-labelledby="drawer-title"
        tabIndex={-1}
      >
        <header className="drawer__head">
          <span className="key">{task.data?.key ?? '…'}</span>
          <span className="shell__spacer" />
          <button type="button" className="button button--quiet" onClick={close}>
            Close
          </button>
        </header>

        {task.isPending ? <p className="empty">Loading task…</p> : null}
        {task.error != null ? (
          <div className="drawer__section">
            <ErrorNotice error={task.error} />
          </div>
        ) : null}

        {task.data === undefined ? null : (
          <div className="drawer__body">
            <h2 id="drawer-title" className="visually-hidden">
              {task.data.key} — {task.data.title}
            </h2>

            <TransitionControl task={task.data} />
            <TaskFields task={task.data} />
            <Assignees task={task.data} />

            <section className="drawer__section" aria-labelledby="drawer-meta">
              <h3 id="drawer-meta" className="drawer__section-title">
                Details
              </h3>
              <dl className="meta">
                <dt>Created</dt>
                <dd>{formatDateTime(task.data.created_at)}</dd>
                <dt>Updated</dt>
                <dd>{formatDateTime(task.data.updated_at)}</dd>
                <dt>Reporter</dt>
                <dd className="key">{task.data.reporter_id}</dd>
                <dt>Version</dt>
                <dd>{task.data.version}</dd>
              </dl>
            </section>

            <CommentThread taskId={task.data.id} />

            <section className="drawer__section" aria-labelledby="drawer-unbuilt">
              <h3 id="drawer-unbuilt" className="drawer__section-title">
                Relations and activity
              </h3>
              {/* Shown rather than omitted: a panel that is simply absent looks
                  like a product that never had the feature, and the next person
                  to open this file cannot tell which. */}
              <GapNotice what="Relations are not readable yet." tracker="C-008">
                <span>
                  <code>POST /api/v1/tasks/&#123;id&#125;/dependencies</code> is specified in
                  docs/05 and no route is registered for it.
                </span>
              </GapNotice>
              <GapNotice what="Activity is not readable yet." tracker="C-011">
                <span>
                  Every change already writes an activity record in the same transaction;
                  <code> GET /api/v1/tasks/&#123;id&#125;/activity</code> is not served yet.
                </span>
              </GapNotice>
            </section>
          </div>
        )}
      </div>
    </div>
  )
}
