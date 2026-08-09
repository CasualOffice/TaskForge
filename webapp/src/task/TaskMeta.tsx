/**
 * The metadata column: everything a reader needs to answer "what is this, and
 * who owns it", in one view.
 *
 * # The failure this module prevents
 *
 * A detail surface you have to scroll to learn the assignee from.
 * `design/DESIGN-FOUNDATION.md` §1.8 — "a surface that scrolls to reveal a field
 * has hidden that field" — and §11 now names "essential fields below the fold on
 * a detail surface" as an anti-pattern. This column is therefore **short by
 * construction**: one line per field, no field with its own heading, nothing
 * behind a disclosure. If it ever needs to scroll, something belongs in the left
 * column instead.
 *
 * # Values are text until you decide to change them
 *
 * Every field reads as a value and acts as a control: priority says `Urgent` and
 * opens five choices when pressed. That is one click to open and one to pick,
 * against three for a form that has to be entered and saved — and at rest the
 * column is legible rather than being a stack of inputs. `docs/23` keeps status
 * out of this pattern: it is a command, not a field (see `StatusControl`).
 *
 * # The assignee gap, stated once
 *
 * `TaskView` carries no `assignees` today and there is no `GET` for them; the
 * write half exists and answers with the resulting set. So this shows the set it
 * has been told about and says plainly when it has not been told — a wrong fact
 * stated confidently is worse than a missing one stated plainly. It reads
 * `task.assignees` when the field appears, so the day the read lands this
 * lights up with no change here.
 */
import { type ReactElement } from 'react'
import { useMutation, useQuery } from '@tanstack/react-query'

import { keys } from '../api/keys'
import { PERMISSIONS } from '../api/permissions'
import {
  PRIORITIES,
  TASK_TYPES,
  type Priority,
  type Task,
  type TaskType,
} from '../api/tasks'
import { assignTask, unassignTask } from '../api/tasks'
import { directory, listMembers } from '../api/workspaces'
import { useAnnounce } from '../shell/announce'
import type { Authority } from '../shell/permissions'
import { ChoiceList, Popover } from '../shell/Popover'
import { useWorkspaceId } from '../shell/session'
import { useTaskPatch } from '../tasks/mutations'
import {
  formatDate,
  formatRelative,
  fromDateInput,
  isOverdue,
  priorityLabel,
  toDateInput,
  typeLabel,
} from '../tasks/present'
import { StatusControl } from './StatusControl'

/** The assignee set when the representation carries one. See the module docs. */
function assigneesOf(task: Task): readonly string[] | undefined {
  return (task as { assignees?: readonly string[] }).assignees
}

export function TaskMeta({
  task,
  authority,
  projectName,
}: {
  task: Task
  authority: Authority
  projectName: string | undefined
}): ReactElement {
  const workspaceId = useWorkspaceId()
  const announce = useAnnounce()
  const patch = useTaskPatch(workspaceId)
  const mayEdit = authority.can(PERMISSIONS.taskUpdate)

  const members = useQuery({
    queryKey: keys.members(workspaceId),
    queryFn: ({ signal }) => listMembers(workspaceId, signal),
    enabled: workspaceId !== '',
    staleTime: 5 * 60_000,
  })
  const nameOf = directory(members.data?.data ?? [])

  function save(fields: Parameters<typeof patch.mutate>[0]['patch'], said: string): void {
    patch.mutate(
      { task, patch: fields },
      {
        onSuccess: () => announce(said),
        onError: () => announce(`${task.key} was not changed.`, 'error'),
      },
    )
  }

  return (
    <dl className="meta2">
      <Row label="Status">
        <StatusControl task={task} authority={authority} />
      </Row>

      <Row label="Assignees">
        <AssigneeField task={task} authority={authority} nameOf={nameOf} />
      </Row>

      <Row label="Priority">
        <Choice
          value={task.priority}
          render={priorityLabel}
          options={PRIORITIES}
          disabled={!mayEdit}
          className={task.priority === 'NONE' ? 'meta2__unset' : `prio prio--${task.priority}`}
          onChoose={(next) =>
            save({ priority: next as Priority }, `Priority set to ${priorityLabel(next)}`)
          }
        />
      </Row>

      <Row label="Type">
        <Choice
          value={task.type}
          render={typeLabel}
          options={TASK_TYPES}
          disabled={!mayEdit}
          className=""
          onChoose={(next) => save({ type: next as TaskType }, `Type set to ${typeLabel(next)}`)}
        />
      </Row>

      <Row label="Due">
        <DateValue
          id="meta-due"
          value={task.due_at}
          late={isOverdue(task)}
          disabled={!mayEdit}
          onChange={(next) => save({ due_at: next }, next === null ? 'Due date cleared' : 'Due date saved')}
        />
      </Row>

      <Row label="Start">
        <DateValue
          id="meta-start"
          value={task.start_at}
          late={false}
          disabled={!mayEdit}
          onChange={(next) => save({ start_at: next }, next === null ? 'Start date cleared' : 'Start date saved')}
        />
      </Row>

      <Row label="Project">
        <span>{projectName ?? '—'}</span>
      </Row>

      <Row label="Reporter">
        <span>{nameOf(task.reporter_id)}</span>
      </Row>

      <Row label="Created">
        <span title={task.created_at}>{formatRelative(task.created_at)}</span>
      </Row>

      <Row label="Updated">
        <span title={task.updated_at}>{formatRelative(task.updated_at)}</span>
      </Row>
    </dl>
  )
}

function Row({ label, children }: { label: string; children: ReactElement }): ReactElement {
  return (
    <div className="meta2__row">
      <dt className="meta2__label">{label}</dt>
      <dd className="meta2__value">{children}</dd>
    </div>
  )
}

/** A value that reads as text and opens its choices when pressed. */
function Choice({
  value,
  options,
  render,
  disabled,
  className,
  onChoose,
}: {
  value: string
  options: readonly string[]
  render: (value: string) => string
  disabled: boolean
  className: string
  onChoose: (next: string) => void
}): ReactElement {
  if (disabled) return <span className={className}>{render(value)}</span>
  return (
    <Popover
      label={render(value)}
      triggerClass={`meta2__button ${className}`}
      align="end"
    >
      {(close) => (
        <ChoiceList
          options={options.map((option) => ({ value: option, label: render(option) }))}
          current={value}
          onChoose={onChoose}
          close={close}
        />
      )}
    </Popover>
  )
}

/**
 * A date that reads as a date.
 *
 * The native picker is the control — every platform in `docs/18` has one, it
 * handles locale and keyboard entry, and a hand-rolled calendar is a month of
 * accessibility bugs. It is revealed by pressing the value rather than sitting
 * open, so the column reads as text at rest.
 */
function DateValue({
  id,
  value,
  late,
  disabled,
  onChange,
}: {
  id: string
  value: string | null
  late: boolean
  disabled: boolean
  onChange: (next: string | null) => void
}): ReactElement {
  const shown = value === null ? 'None' : formatDate(value)
  if (disabled) return <span className={value === null ? 'meta2__unset' : ''}>{shown}</span>
  return (
    <Popover
      label={shown}
      ariaLabel={`${shown}. Change date.`}
      align="end"
      triggerClass={`meta2__button${late ? ' meta2__button--late' : ''}${value === null ? ' meta2__unset' : ''}`}
    >
      {(close) => (
        <div className="field meta2__date">
          <label className="field__label" htmlFor={id}>
            Date
          </label>
          <input
            id={id}
            className="input"
            type="date"
            value={toDateInput(value)}
            onChange={(event) => {
              onChange(fromDateInput(event.target.value))
              close()
            }}
          />
          {value === null ? null : (
            <button
              type="button"
              className="button button--quiet"
              onClick={() => {
                onChange(null)
                close()
              }}
            >
              Clear
            </button>
          )}
        </div>
      )}
    </Popover>
  )
}

function AssigneeField({
  task,
  authority,
  nameOf,
}: {
  task: Task
  authority: Authority
  nameOf: (id: string) => string
}): ReactElement {
  const workspaceId = useWorkspaceId()
  const announce = useAnnounce()
  const mayAssign = authority.can(PERMISSIONS.taskAssign)
  const known = assigneesOf(task)

  const members = useQuery({
    queryKey: keys.members(workspaceId),
    queryFn: ({ signal }) => listMembers(workspaceId, signal),
    enabled: workspaceId !== '',
    staleTime: 5 * 60_000,
  })

  const assign = useMutation({
    // The two endpoints answer with different bodies — the assign returns the
    // resulting set, the unassign returns nothing — and neither is used here,
    // so the result is discarded rather than widened into a union nobody reads.
    mutationFn: async ({ userId, on }: { userId: string; on: boolean }): Promise<void> => {
      if (on) await assignTask(workspaceId, task.id, userId)
      else await unassignTask(workspaceId, task.id, userId)
    },
    onSuccess: (_result, { userId, on }) =>
      announce(`${nameOf(userId)} ${on ? 'assigned to' : 'unassigned from'} ${task.key}`),
    onError: () => announce('The assignment did not save.', 'error'),
  })

  const people = members.data?.data ?? []
  const label =
    known === undefined
      ? 'Not shown yet'
      : known.length === 0
        ? 'Nobody'
        : known.map(nameOf).join(', ')

  const value = (
    <span className={known === undefined || known.length === 0 ? 'meta2__unset' : ''}>{label}</span>
  )

  if (!mayAssign) return value

  return (
    <Popover
      label={value}
      ariaLabel={`Assignees: ${label}. Change.`}
      align="end"
      triggerClass="meta2__button"
    >
      {() => (
        <div>
          {known === undefined ? (
            /* One quiet line, not a boxed developer notice: the reader is about
               to assign someone and needs to know the list they see afterwards
               is this session's answer, not the server's. */
            <p className="gapline pop__section">
              This list only shows changes made here — the server does not return a
              task’s assignees yet.
            </p>
          ) : null}
          <ul className="pop__list">
            {people.map((member) => {
              const on = (known ?? []).includes(member.user_id)
              return (
                <li key={member.user_id}>
                  <button
                    type="button"
                    className="pop__item"
                    aria-current={on ? 'true' : undefined}
                    disabled={assign.isPending}
                    onClick={() => assign.mutate({ userId: member.user_id, on: !on })}
                  >
                    {member.display_name}
                  </button>
                </li>
              )
            })}
            {people.length === 0 ? <li className="pop__section">No members to assign.</li> : null}
          </ul>
        </div>
      )}
    </Popover>
  )
}
