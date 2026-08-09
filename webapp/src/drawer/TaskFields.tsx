/**
 * Editing a task's plain fields.
 *
 * # The failure this module prevents
 *
 * Losing what someone typed. docs/42 §Optimistic mutation, rule 3: "Failure
 * never discards user input." So each field holds a *draft*, the draft is only
 * cleared by a successful write, and a refusal leaves the text on screen with
 * the reason beside it. The alternative — re-deriving the input from the server
 * copy on every render — silently reverts a paragraph the moment a `409`
 * arrives, which is the single most infuriating bug a tracker can have.
 *
 * # Why status is not here
 *
 * `docs/23`: status is never written directly, and `PATCH` refuses it with
 * `TF-WFL-0001`. The type in `api/tasks.ts` cannot express it, so this file
 * *could not* offer the control even by mistake — see `TransitionControl`.
 *
 * # Progressive disclosure
 *
 * docs/42 §Progressive disclosure names this the highest-leverage decision in
 * the UI. Title and description are always visible; type, priority and dates sit
 * behind one disclosure, because "every tracker that opens with fourteen fields
 * trains its users to paste 'TODO' into half of them".
 */
import { useEffect, useState, type ReactElement } from 'react'

import { PERMISSIONS } from '../api/permissions'
import { PRIORITIES, TASK_TYPES, type Priority, type Task, type TaskType } from '../api/tasks'
import { useAnnounce } from '../shell/announce'
import type { Authority } from '../shell/permissions'
import { ErrorNotice } from '../shell/notice'
import { useWorkspaceId } from '../shell/session'
import { fromDateInput, priorityLabel, toDateInput, typeLabel } from '../tasks/present'
import { useTaskPatch } from '../tasks/mutations'

export function TaskFields({
  task,
  authority,
}: {
  task: Task
  authority: Authority
}): ReactElement {
  const workspaceId = useWorkspaceId()
  // `docs/04` resolves this; the client only asks. A `conditional` grant counts
  // as permission — only the server can evaluate the constraint for this task,
  // and hiding it would hide the reporter's own edit on the task they reported.
  const mayEdit = authority.can(PERMISSIONS.taskUpdate)
  const announce = useAnnounce()
  const patch = useTaskPatch(workspaceId)

  const [title, setTitle] = useState(task.title)
  const [description, setDescription] = useState(task.description ?? '')
  const [open, setOpen] = useState(false)

  // Adopt the server's copy only when this drawer changes task. Syncing on every
  // change of `task` would overwrite a draft the moment an unrelated field was
  // saved, which is the input-discarding bug in a subtler costume.
  useEffect(() => {
    setTitle(task.title)
    setDescription(task.description ?? '')
  }, [task.id, task.title, task.description])

  const titleDirty = title.trim() !== task.title
  const descriptionDirty = description !== (task.description ?? '')

  function save(fields: Parameters<typeof patch.mutate>[0]['patch'], said: string): void {
    patch.mutate(
      { task, patch: fields },
      { onSuccess: () => announce(said) },
    )
  }

  return (
    <div className="fields">
      {mayEdit ? null : (
        <p className="field__hint">You can read this task but not change it.</p>
      )}

      <div className="field">
        <label className="field__label" htmlFor="task-title">
          Title
        </label>
        <input
          id="task-title"
          className="input drawer__title-input"
          value={title}
          readOnly={!mayEdit}
          onChange={(event) => setTitle(event.target.value)}
          onBlur={() => {
            if (titleDirty && title.trim() !== '') save({ title: title.trim() }, 'Title saved')
          }}
        />
      </div>

      <div className="field">
        <label className="field__label" htmlFor="task-description">
          Description
        </label>
        <textarea
          id="task-description"
          className="textarea"
          rows={5}
          value={description}
          readOnly={!mayEdit}
          onChange={(event) => setDescription(event.target.value)}
        />
        {descriptionDirty && mayEdit ? (
          <div className="field__actions">
            <button
              type="button"
              className="button button--primary"
              disabled={patch.isPending}
              onClick={() =>
                save({ description: description === '' ? null : description }, 'Description saved')
              }
            >
              Save description
            </button>
            <button
              type="button"
              className="button button--quiet"
              onClick={() => setDescription(task.description ?? '')}
            >
              Discard
            </button>
          </div>
        ) : null}
      </div>

      <button
        type="button"
        className="button button--quiet drawer__disclose"
        aria-expanded={open}
        onClick={() => setOpen(!open)}
      >
        {open ? 'Hide details' : 'Show details'}
      </button>

      {open ? (
        <div className="drawer__details">
          <Choice
            id="task-type"
            label="Type"
            value={task.type}
            options={TASK_TYPES}
            render={typeLabel}
            disabled={!mayEdit}
            onChange={(next) => save({ type: next as TaskType }, `Type set to ${typeLabel(next)}`)}
          />
          <Choice
            id="task-priority"
            label="Priority"
            value={task.priority}
            options={PRIORITIES}
            render={priorityLabel}
            disabled={!mayEdit}
            onChange={(next) =>
              save({ priority: next as Priority }, `Priority set to ${priorityLabel(next)}`)
            }
          />
          <DateField
            id="task-start"
            label="Start"
            value={task.start_at}
            disabled={!mayEdit}
            onChange={(next) => save({ start_at: next }, 'Start date saved')}
          />
          <DateField
            id="task-due"
            label="Due"
            value={task.due_at}
            disabled={!mayEdit}
            onChange={(next) => save({ due_at: next }, 'Due date saved')}
          />
        </div>
      ) : null}

      {patch.isError ? <ErrorNotice error={patch.error} /> : null}
    </div>
  )
}

function Choice({
  id,
  label,
  value,
  options,
  render,
  disabled,
  onChange,
}: {
  id: string
  label: string
  value: string
  options: readonly string[]
  render: (value: string) => string
  disabled: boolean
  onChange: (next: string) => void
}): ReactElement {
  return (
    <div className="field">
      <label className="field__label" htmlFor={id}>
        {label}
      </label>
      <select
        id={id}
        className="select"
        value={value}
        disabled={disabled}
        onChange={(event) => onChange(event.target.value)}
      >
        {options.map((option) => (
          <option key={option} value={option}>
            {render(option)}
          </option>
        ))}
      </select>
    </div>
  )
}

function DateField({
  id,
  label,
  value,
  disabled,
  onChange,
}: {
  id: string
  label: string
  value: string | null
  disabled: boolean
  onChange: (next: string | null) => void
}): ReactElement {
  return (
    <div className="field">
      <label className="field__label" htmlFor={id}>
        {label}
      </label>
      <input
        id={id}
        className="input"
        type="date"
        disabled={disabled}
        value={toDateInput(value)}
        onChange={(event) => onChange(fromDateInput(event.target.value))}
      />
    </div>
  )
}
