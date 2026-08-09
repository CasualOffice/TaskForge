/**
 * Changing a task's status — the only door there is, as one control.
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
 *
 * # Why it is one control and not N buttons
 *
 * The previous shape rendered a row of "Move to Backlog", "Move to In Progress"
 * buttons — `design/LAYOUT-AND-INTERACTION-GUIDELINES.md` §5: "Do not fill every
 * toolbar with text buttons", and §8 asks a surface to answer *where am I*
 * before *what can I do next*. N buttons answer the second and never the first:
 * the current status was a separate chip somewhere else, and the eye landed on
 * the verbs. A trigger that shows the current status and opens the moves out of
 * it answers both, in the same square inch, in one click.
 *
 * # Why the note is behind a disclosure
 *
 * It was the first interactive element on the whole surface, so it was where the
 * eye landed — on an optional field for a comment nobody had decided to write
 * yet. It is offered where it is used: inside the menu, under the moves.
 */
import { useState, type ReactElement } from 'react'
import { Input } from '@schnsrw/design-system'

import { PERMISSIONS } from '../api/permissions'
import type { Task } from '../api/tasks'
import { useAnnounce } from '../shell/announce'
import type { Authority } from '../shell/permissions'
import { Popover } from '../shell/Popover'
import { useWorkspaceId } from '../shell/session'
import { useTaskTransition } from '../tasks/mutations'
import { stateLabel } from '../tasks/present'
import { useProjectWorkflow } from '../tasks/useWorkflow'

export function StatusControl({
  task,
  authority,
}: {
  task: Task
  authority: Authority
}): ReactElement {
  const workspaceId = useWorkspaceId()
  const announce = useAnnounce()
  const { workflow, unavailable: missingWorkflow } = useProjectWorkflow(task.project_id)
  const move = useTaskTransition(workspaceId)
  const [note, setNote] = useState('')
  const [noteOpen, setNoteOpen] = useState(false)

  const mayTransition = authority.can(PERMISSIONS.taskTransition)
  // Two different reasons the control cannot be used, kept apart: one is an
  // unbuilt endpoint and one is an authority decision. Collapsing them into a
  // disabled button would tell a user without `task.transition` that the server
  // is incomplete, and an admin that they lack a grant.
  const why = mayTransition ? missingWorkflow : 'You cannot change this task’s status.'

  const current = workflow?.statuses.find((status) => status.id === task.status_id)
  const label = current?.name ?? stateLabel(task.state)

  // Each edge out of the current status, with the status it leads to and
  // whether this actor may take it.
  //
  // A transition carries its OWN `required_permission` on top of
  // `task.transition` — `docs/23` step 5 — so an actor who may move a task in
  // general may still not make one particular move. Rendering the entry disabled
  // with the permission named is a better refusal than a `TF-WFL-0003` after the
  // click, and it is why the server returns the field rather than filtering the
  // edge out itself.
  const targets =
    workflow === undefined
      ? []
      : workflow.transitions
          .filter((edge) => edge.from === task.status_id)
          .flatMap((edge) => {
            const to = workflow.statuses.find((status) => status.id === edge.to)
            if (to === undefined) return []
            const needed = edge.required_permission
            const permitted = needed === null || authority.can(needed)
            return [{ status: to, permitted, needed }]
          })

  const chip = (
    <>
      <span className={`dot dot--${task.state}`} aria-hidden="true" />
      <span className="statusctl__name">{label}</span>
      {why === undefined ? (
        <span className="statusctl__caret" aria-hidden="true">
          ▾
        </span>
      ) : null}
    </>
  )

  // A status that cannot be changed is still a status, so it is still shown —
  // as text rather than as a dead button. A disabled control that looks like a
  // control is a control the user will press repeatedly.
  if (why !== undefined) {
    return (
      <span className="statusctl statusctl--static" title={why}>
        {chip}
      </span>
    )
  }

  return (
    <Popover label={chip} ariaLabel={`Status: ${label}. Change status.`} triggerClass="statusctl">
      {(close) => (
        <div className="statusctl__menu">
          <p className="pop__section">Move to</p>
          {targets.length === 0 ? (
            <p className="field__hint pop__section">This workflow has no move out of {label}.</p>
          ) : (
            <ul className="pop__list">
              {targets.map(({ status: target, permitted, needed }) => (
                <li key={target.id}>
                  <button
                    type="button"
                    className="pop__item"
                    disabled={move.isPending || !permitted}
                    title={permitted ? undefined : `This move needs ${needed}.`}
                    onClick={() => {
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
                            setNoteOpen(false)
                            announce(`${task.key} moved to ${target.name}`)
                          },
                          onError: () =>
                            announce(`${task.key} did not move to ${target.name}.`, 'error'),
                        },
                      )
                      close()
                    }}
                  >
                    <span className={`dot dot--${target.state}`} aria-hidden="true" />
                    {target.name}
                    {permitted ? null : <span className="pop__why">no permission</span>}
                  </button>
                </li>
              ))}
            </ul>
          )}

          <div className="pop__divider" />
          {noteOpen ? (
            <div className="field statusctl__note">
              <label className="field__label" htmlFor="transition-note">
                Note
              </label>
              <Input
                full
                id="transition-note"
                value={note}
                onChange={(event) => setNote(event.target.value)}
              />
              <span className="field__hint">Posted as a comment with the move.</span>
            </div>
          ) : (
            <button type="button" className="pop__item" onClick={() => setNoteOpen(true)}>
              Add a note with the move…
            </button>
          )}
        </div>
      )}
    </Popover>
  )
}
