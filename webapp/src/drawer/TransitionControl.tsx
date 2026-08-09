/**
 * Changing a task's status — the only door there is.
 *
 * # The failure this module prevents
 *
 * A status dropdown that writes the field. `docs/23`: "Status is never written
 * directly. Transitions are commands." A `<select>` bound to `status_id` would
 * be one `PATCH` away from bypassing transition validity, required fields,
 * dependency gating, and the transition's own permission — four rules the server
 * enforces and a client must not appear to offer a way around.
 *
 * So the control offers *transitions*, not statuses: the options come from the
 * workflow's edges out of the current status, and choosing one issues the
 * command with the task's version as `If-Match`.
 */
import { useState, type ReactElement } from 'react'

import { PERMISSIONS } from '../api/permissions'
import type { Task } from '../api/tasks'
import { useAnnounce } from '../shell/announce'
import { ErrorNotice, GapNotice } from '../shell/notice'
import type { Authority } from '../shell/permissions'
import { useWorkspaceId } from '../shell/session'
import { useTaskTransition } from '../tasks/mutations'
import { stateLabel } from '../tasks/present'
import { useProjectWorkflow } from '../tasks/useWorkflow'

export function TransitionControl({
  task,
  authority,
}: {
  task: Task
  authority: Authority
}): ReactElement {
  const workspaceId = useWorkspaceId()
  const announce = useAnnounce()
  const { workflow, unavailable: missingWorkflow } = useProjectWorkflow(task.project_id)
  // Two different reasons the control cannot be used, kept apart: one is an
  // unbuilt endpoint and one is an authority decision. Collapsing them into a
  // disabled button would tell a user without `task.transition` that the server
  // is incomplete, and an admin that they lack a grant.
  const unavailable = authority.can(PERMISSIONS.taskTransition)
    ? missingWorkflow
    : 'You do not have permission to change this task’s status.'
  const move = useTaskTransition(workspaceId)
  const [note, setNote] = useState('')

  const current = workflow?.statuses.find((status) => status.id === task.status_id)

  const targets =
    workflow === undefined
      ? []
      : workflow.transitions
          .filter((edge) => edge.from === task.status_id)
          .flatMap((edge) => {
            const to = workflow.statuses.find((status) => status.id === edge.to)
            return to === undefined ? [] : [to]
          })

  return (
    <section className="drawer__section" aria-labelledby="transition-heading">
      <h3 id="transition-heading" className="drawer__section-title">
        Status
      </h3>

      <p className="drawer__status">
        <span className={`pill pill--${task.state}`}>
          {current?.name ?? stateLabel(task.state)}
        </span>
      </p>

      {unavailable === undefined ? (
        <>
          <div className="field">
            <label className="field__label" htmlFor="transition-note">
              Note (optional)
            </label>
            <input
              id="transition-note"
              className="input"
              value={note}
              onChange={(event) => setNote(event.target.value)}
            />
            <span className="field__hint">Written as a comment in the same transaction.</span>
          </div>

          <div className="drawer__transitions">
            {targets.length === 0 ? (
              <p className="field__hint">This workflow has no move out of the current status.</p>
            ) : (
              targets.map((target) => (
                <button
                  key={target.id}
                  type="button"
                  className="button"
                  disabled={move.isPending}
                  onClick={() =>
                    move.mutate(
                      {
                        task,
                        toStatusId: target.id,
                        toState: target.state,
                        ...(note.trim() === '' ? {} : { comment: note.trim() }),
                      },
                      {
                        onSuccess: () => {
                          setNote('')
                          announce(`${task.key} moved to ${target.name}`)
                        },
                      },
                    )
                  }
                >
                  Move to {target.name}
                </button>
              ))
            )}
          </div>
        </>
      ) : (
        <GapNotice what="Status cannot be changed from here yet." tracker="D-059">
          <span>{unavailable}</span>
        </GapNotice>
      )}

      {move.isError ? <ErrorNotice error={move.error} /> : null}
    </section>
  )
}
