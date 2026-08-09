/**
 * Creating a task.
 *
 * # The failure this module prevents
 *
 * A tracker you cannot add work to. This control used to return `null` whenever
 * no project was scoped — which is the state the application *lands in*, because
 * the list opens on "All projects". The result was a task tracker whose default
 * screen had no way to create a task, and the reason was invisible: nothing was
 * disabled and nothing was explained, the button simply was not there.
 *
 * A missing project is a question, not a disqualification. So the control is
 * always offered, and the form asks which project — required field first, per
 * `design/LAYOUT-AND-INTERACTION-GUIDELINES.md` §6 — defaulting to whatever the
 * view is scoped to so the common case is still one field and one Enter.
 *
 * # A popover, not a modal
 *
 * §6: "Avoid modal forms for workflows that require referencing the underlying
 * task/project." Someone naming a task is usually reading the board behind it.
 *
 * # Why the second field set is behind a disclosure
 *
 * docs/42 §Progressive disclosure calls this "the single highest-leverage
 * decision in the UI" and says why: "Every tracker that opens with fourteen
 * fields trains its users to paste 'TODO' into half of them." Title is the
 * required field; type and priority are one click away for the person who
 * already knows the answer; everything else is the detail surface's job.
 *
 * # Why the idempotency key is minted once per attempt, not per submit
 *
 * `docs/24` requires `Idempotency-Key` on create, and the point of it is that a
 * retry of the *same* attempt does not produce a second task. A key minted per
 * click would make a double-click two tasks; one minted once and never rotated
 * would make the user's *next* task a replay of their last. It is therefore
 * rotated on success, which is exactly when the attempt is over.
 */
import { Button, Input, Select } from '@schnsrw/design-system'
import { useEffect, useRef, useState, type FormEvent, type ReactElement } from 'react'
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query'

import { idempotencyKey } from '../api/http'
import { keys } from '../api/keys'
import { PERMISSIONS } from '../api/permissions'
import { listProjects } from '../api/projects'
import { createTask, PRIORITIES, TASK_TYPES, type Priority, type TaskType } from '../api/tasks'
import { useAnnounce } from '../shell/announce'
import { useOpenTask } from '../shell/navigation'
import { ErrorNotice } from '../shell/notice'
import { useAuthority } from '../shell/permissions'
import { Popover } from '../shell/Popover'
import { useWorkspaceId } from '../shell/session'
import { priorityLabel, typeLabel } from '../tasks/present'

export function CreateTask({
  projectId,
  variant = 'primary',
  label = 'New task',
}: {
  /** The project the view is scoped to, used as the default. May be absent. */
  projectId: string | undefined
  /** `quiet` for the copy inside an empty state, which already carries the emphasis. */
  variant?: 'primary' | 'quiet'
  label?: string
}): ReactElement | null {
  // Asked at workspace scope when nothing is scoped, which is the honest
  // question: "may this person create a task anywhere here". A project-scoped
  // refusal still surfaces — the server re-authorizes the write regardless, and
  // its refusal carries a registry code the notice names.
  const authority = useAuthority(projectId)
  if (!authority.can(PERMISSIONS.taskCreate)) return null

  return (
    <Popover
      label={label}
      align="end"
      triggerVariant={variant === 'primary' ? 'primary' : 'subtle'}
    >
      {(close) => <CreateForm projectId={projectId} close={close} />}
    </Popover>
  )
}

function CreateForm({
  projectId,
  close,
}: {
  projectId: string | undefined
  close: () => void
}): ReactElement {
  const workspaceId = useWorkspaceId()
  const announce = useAnnounce()
  const openTask = useOpenTask()
  const client = useQueryClient()

  const projects = useQuery({
    queryKey: keys.projects(workspaceId),
    queryFn: ({ signal }) => listProjects(workspaceId, signal),
    enabled: workspaceId !== '',
    staleTime: 60_000,
  })

  const available = projects.data?.data ?? []
  const [project, setProject] = useState(projectId ?? '')
  const [title, setTitle] = useState('')
  const [type, setType] = useState<TaskType>('TASK')
  const [priority, setPriority] = useState<Priority>('NONE')
  const [more, setMore] = useState(false)
  const attempt = useRef(idempotencyKey())
  const titleRef = useRef<HTMLInputElement>(null)

  // The surface exists because the user pressed "New task", so focus belongs in
  // the field they came for. Done with a ref rather than `autoFocus`, which is
  // the page-load version of the same idea and which `jsx-a11y` is right about.
  useEffect(() => titleRef.current?.focus(), [])

  // Only when the caller passed none and exactly one exists: choosing *for* the
  // user out of several would silently file work in the wrong place.
  const chosen = project !== '' ? project : available.length === 1 ? (available[0]?.id ?? '') : ''

  const create = useMutation({
    mutationFn: () =>
      createTask(
        workspaceId,
        chosen,
        {
          title: title.trim(),
          ...(type === 'TASK' ? {} : { type }),
          ...(priority === 'NONE' ? {} : { priority }),
        },
        attempt.current,
      ),
    onSuccess: (task) => {
      attempt.current = idempotencyKey()
      announce(`Created ${task.key} — ${task.title}`)
      void client.invalidateQueries({ queryKey: keys.taskLists(workspaceId) })
      close()
      openTask(task.id)
    },
  })

  function submit(event: FormEvent): void {
    event.preventDefault()
    if (title.trim() !== '' && chosen !== '') create.mutate()
  }

  if (!projects.isPending && available.length === 0) {
    return (
      <div className="create">
        <p className="state__title">There are no projects yet.</p>
        <p className="state__detail">
          A task belongs to a project. Create a project first, or ask an owner to.
        </p>
      </div>
    )
  }

  return (
    <form className="create" onSubmit={submit}>
      {/* Required field first (§6), and the label stays visible — §6 again:
          "placeholder text is not a label". */}
      <div className="field">
        <label className="field__label" htmlFor="create-project">
          Project
        </label>
        <Select
          full
          id="create-project"
          value={chosen}
          onChange={(event) => setProject(event.target.value)}
        >
          <option value="">Choose a project…</option>
          {available.map((entry) => (
            <option key={entry.id} value={entry.id}>
              {entry.key} — {entry.name}
            </option>
          ))}
        </Select>
      </div>

      <div className="field">
        <label className="field__label" htmlFor="create-title">
          Title
        </label>
        <Input
          full
          id="create-title"
          ref={titleRef}
          value={title}
          onChange={(event) => setTitle(event.target.value)}
        />
      </div>

      <Button
        variant="subtle"
        className="create__more"
        aria-expanded={more}
        onClick={() => setMore(!more)}
      >
        {more ? 'Fewer fields' : 'Type and priority'}
      </Button>

      {more ? (
        <div className="create__row">
          <div className="field">
            <label className="field__label" htmlFor="create-type">
              Type
            </label>
            <Select
              full
              id="create-type"
              value={type}
              onChange={(event) => setType(event.target.value as TaskType)}
            >
              {TASK_TYPES.map((option) => (
                <option key={option} value={option}>
                  {typeLabel(option)}
                </option>
              ))}
            </Select>
          </div>
          <div className="field">
            <label className="field__label" htmlFor="create-priority">
              Priority
            </label>
            <Select
              full
              id="create-priority"
              value={priority}
              onChange={(event) => setPriority(event.target.value as Priority)}
            >
              {PRIORITIES.map((option) => (
                <option key={option} value={option}>
                  {priorityLabel(option)}
                </option>
              ))}
            </Select>
          </div>
        </div>
      ) : null}

      <div className="field__actions">
        <Button
          variant="primary"
          type="submit"
          disabled={create.isPending || title.trim() === '' || chosen === ''}
        >
          {create.isPending ? 'Creating…' : 'Create task'}
        </Button>
        <Button variant="subtle" onClick={close}>
          Cancel
        </Button>
      </div>

      {/* Beside the fields, not over them: §6 puts validation near what it is
          about, and the draft is still on screen to correct. */}
      {create.isError ? <ErrorNotice error={create.error} /> : null}
    </form>
  )
}
