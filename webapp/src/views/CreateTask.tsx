/**
 * Creating a task: one field.
 *
 * # The failure this module prevents
 *
 * A create form with fourteen fields. docs/42 §Progressive disclosure calls this
 * "the single highest-leverage decision in the UI" and says why: "Every tracker
 * that opens with fourteen fields trains its users to paste 'TODO' into half of
 * them." So this asks for a title, the server defaults everything else, and the
 * rest of the task is edited in the drawer that opens straight afterwards.
 *
 * # Why the idempotency key is minted once per attempt, not per submit
 *
 * `docs/24` requires `Idempotency-Key` on create, and the point of it is that a
 * retry of the *same* attempt does not produce a second task. A key minted per
 * click would make a double-click two tasks; one minted once and never rotated
 * would make the user's *next* task a replay of their last. It is therefore
 * rotated on success, which is exactly when the attempt is over.
 */
import { useRef, useState, type FormEvent, type ReactElement } from 'react'
import { useMutation, useQueryClient } from '@tanstack/react-query'

import { idempotencyKey } from '../api/http'
import { keys } from '../api/keys'
import { PERMISSIONS } from '../api/permissions'
import { createTask } from '../api/tasks'
import { useAnnounce } from '../shell/announce'
import { useOpenTask } from '../shell/navigation'
import { ErrorNotice } from '../shell/notice'
import { useAuthority } from '../shell/permissions'
import { useWorkspaceId } from '../shell/session'

export function CreateTask({ projectId }: { projectId: string | undefined }): ReactElement | null {
  const workspaceId = useWorkspaceId()
  const authority = useAuthority(projectId)
  const announce = useAnnounce()
  const openTask = useOpenTask()
  const client = useQueryClient()

  const [open, setOpen] = useState(false)
  const [title, setTitle] = useState('')
  const attempt = useRef(idempotencyKey())

  const create = useMutation({
    mutationFn: () => createTask(workspaceId, projectId ?? '', { title: title.trim() }, attempt.current),
    onSuccess: (task) => {
      attempt.current = idempotencyKey()
      setTitle('')
      setOpen(false)
      announce(`Created ${task.key}`)
      void client.invalidateQueries({ queryKey: keys.taskLists(workspaceId) })
      openTask(task.id)
    },
  })

  // No project, no create: a task belongs to one, and the endpoint is
  // `POST /projects/{id}/tasks`. Rendering a button that opens a form that
  // cannot submit would be worse than rendering nothing.
  if (projectId === undefined) return null
  if (!authority.can(PERMISSIONS.taskCreate)) return null

  if (!open) {
    return (
      <button type="button" className="button button--primary" onClick={() => setOpen(true)}>
        New task
      </button>
    )
  }

  function submit(event: FormEvent): void {
    event.preventDefault()
    if (title.trim() !== '') create.mutate()
  }

  return (
    <form className="create" onSubmit={submit}>
      <label className="visually-hidden" htmlFor="create-title">
        Task title
      </label>
      <input
        id="create-title"
        className="input create__title"
        value={title}
        placeholder="What needs doing?"
        onChange={(event) => setTitle(event.target.value)}
        onKeyDown={(event) => {
          if (event.key === 'Escape') setOpen(false)
        }}
      />
      <button type="submit" className="button button--primary" disabled={create.isPending}>
        {create.isPending ? 'Creating…' : 'Create'}
      </button>
      <button type="button" className="button button--quiet" onClick={() => setOpen(false)}>
        Cancel
      </button>
      {create.isError ? <ErrorNotice error={create.error} /> : null}
    </form>
  )
}
