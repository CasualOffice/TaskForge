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
 * # What this file owns
 *
 * The shell: the scrim, the dialog semantics, the focus contract, and the
 * *order* of the panels — which it does not choose. The order comes from the
 * extension point registry (`docs/34`, C-017), so the core's own panels go
 * through the same seam a plugin will. Editing lives in `TaskFields`, the thread
 * in `CommentThread`, the status change in `TransitionControl`; they change for
 * three different documents' reasons (`docs/05`, `docs/06`, `docs/23`).
 */
import { useCallback, useRef, type ReactElement } from 'react'
import { useQuery } from '@tanstack/react-query'

import { keys } from '../api/keys'
import { readTask } from '../api/tasks'
import { contributions } from '../extensions/coreContributions'
import { useFocusTrap } from '../shell/focusTrap'
import { useOpenTask } from '../shell/navigation'
import { ErrorNotice } from '../shell/notice'
import { useAuthority } from '../shell/permissions'
import { useWorkspaceId } from '../shell/session'
import { TransitionControl } from './TransitionControl'
import { UnbuiltPanel, panelFor } from './panels'

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

  // Project-scoped, not workspace-scoped: a constrained grant reaches a task
  // through its project, and asking at workspace scope would understate the
  // answer for anyone whose authority comes from a project role.
  const authority = useAuthority(task.data?.project_id)

  const panels = contributions('ui.task.panel')

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
            <h2 id="drawer-title" className="drawer__heading">
              {task.data.title}
            </h2>

            <TransitionControl task={task.data} authority={authority} />

            {panels.map((contribution) => {
              const Panel = panelFor(contribution.slug)
              return (
                <section
                  key={contribution.slug}
                  className="drawer__section"
                  aria-labelledby={`panel-${contribution.slug}`}
                >
                  <h3 id={`panel-${contribution.slug}`} className="drawer__section-title">
                    {contribution.title}
                  </h3>
                  {Panel === undefined ? (
                    <UnbuiltPanel slug={contribution.slug} />
                  ) : (
                    <Panel task={task.data} authority={authority} />
                  )}
                </section>
              )
            })}
          </div>
        )}
      </div>
    </div>
  )
}
