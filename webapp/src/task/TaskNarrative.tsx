/**
 * The title and the description: a detail view first, editable second.
 *
 * # The failure this module prevents
 *
 * Opening a task to *read* it and being handed a form. The previous surface put
 * a live `<input>` where the title should have been and a raw `<textarea>` where
 * the description should have been, so the first thing the eye met was an empty
 * box, there was no visible save model, and a stray keystroke edited the task.
 *
 * The rule here is: **reading is the default state, editing is entered on
 * purpose and left on purpose.** A value is text. Pressing it — or its Edit
 * control — makes it a field with a Save and a Cancel. `Escape` cancels,
 * `Enter` saves the title (a title is one line, so Enter cannot mean newline),
 * and nothing is written on blur, because a write that happens because you
 * clicked elsewhere is a write you did not make.
 *
 * # Failure never discards user input
 *
 * docs/42 §Optimistic mutation, rule 3. The draft lives here and is cleared only
 * by a successful write, so a `409` leaves the text on screen with the reason
 * beside it. Re-deriving the field from the server copy on every render is the
 * bug that silently reverts a paragraph the moment someone else saves.
 */
import { Button, Input } from '@schnsrw/design-system'
import { useEffect, useRef, useState, type ReactElement } from 'react'

import { PERMISSIONS } from '../api/permissions'
import type { Task } from '../api/tasks'
import { useAnnounce } from '../shell/announce'
import { ErrorNotice } from '../shell/notice'
import type { Authority } from '../shell/permissions'
import { useWorkspaceId } from '../shell/session'
import { useTaskPatch } from '../tasks/mutations'

export function TaskTitle({
  task,
  authority,
  as = 'h1',
}: {
  task: Task
  authority: Authority
  /** `h1` on the full route, `h2` in the peek — one heading level per surface. */
  as?: 'h1' | 'h2'
}): ReactElement {
  const workspaceId = useWorkspaceId()
  const announce = useAnnounce()
  const patch = useTaskPatch(workspaceId)
  const mayEdit = authority.can(PERMISSIONS.taskUpdate)

  const [editing, setEditing] = useState(false)
  const [draft, setDraft] = useState(task.title)
  const input = useRef<HTMLInputElement>(null)

  useEffect(() => {
    if (editing) input.current?.select()
  }, [editing])

  // Adopt the server's copy only when the surface changes *task*.
  //
  // Adjusting state during render rather than in an effect, which is React's own
  // answer for "reset when a prop changes": an effect keyed on `task.id` alone
  // is what the exhaustive-deps rule objects to, and the rule's fix — adding
  // `task.title` — is the bug. It would re-adopt the server's value on every
  // refetch and silently discard the sentence someone was halfway through
  // typing. That is exactly rule 3 of docs/42 §Optimistic mutation.
  const shown = useRef(task.id)
  if (shown.current !== task.id) {
    shown.current = task.id
    setDraft(task.title)
    setEditing(false)
  }

  const Heading = as

  function save(): void {
    const next = draft.trim()
    if (next === '' || next === task.title) {
      setEditing(false)
      setDraft(task.title)
      return
    }
    patch.mutate(
      { task, patch: { title: next } },
      {
        onSuccess: () => {
          setEditing(false)
          announce('Title saved')
        },
        onError: () => announce('The title did not save.', 'error'),
      },
    )
  }

  if (!editing) {
    return (
      <div className="narr__titlerow">
        <Heading className="narr__title">{task.title}</Heading>
        {mayEdit ? (
          <Button variant="subtle" className="narr__edit" onClick={() => setEditing(true)}>
            Rename
          </Button>
        ) : null}
      </div>
    )
  }

  return (
    <div className="narr__titleedit">
      <label className="visually-hidden" htmlFor="task-title">
        Title
      </label>
      <Input
        full
        id="task-title"
        ref={input}
        value={draft}
        onChange={(event) => setDraft(event.target.value)}
        onKeyDown={(event) => {
          if (event.key === 'Enter') {
            event.preventDefault()
            save()
          }
          if (event.key === 'Escape') {
            event.stopPropagation()
            setDraft(task.title)
            setEditing(false)
          }
        }}
      />
      <div className="field__actions">
        <Button variant="primary" disabled={patch.isPending || draft.trim() === ''} onClick={save}>
          Save
        </Button>
        <Button
          variant="subtle"
          onClick={() => {
            setDraft(task.title)
            setEditing(false)
          }}
        >
          Cancel
        </Button>
      </div>
      {patch.isError ? <ErrorNotice error={patch.error} /> : null}
    </div>
  )
}

export function TaskDescription({
  task,
  authority,
}: {
  task: Task
  authority: Authority
}): ReactElement {
  const workspaceId = useWorkspaceId()
  const announce = useAnnounce()
  const patch = useTaskPatch(workspaceId)
  const mayEdit = authority.can(PERMISSIONS.taskUpdate)

  const [editing, setEditing] = useState(false)
  const [draft, setDraft] = useState(task.description ?? '')

  // Same reset-on-identity rule as the title; see that comment.
  const shown = useRef(task.id)
  if (shown.current !== task.id) {
    shown.current = task.id
    setDraft(task.description ?? '')
    setEditing(false)
  }

  function save(): void {
    patch.mutate(
      { task, patch: { description: draft === '' ? null : draft } },
      {
        onSuccess: () => {
          setEditing(false)
          announce('Description saved')
        },
        onError: () => announce('The description did not save.', 'error'),
      },
    )
  }

  if (editing) {
    return (
      <section className="narr__section dsec" aria-labelledby="desc-heading">
        <h2 id="desc-heading" className="narr__heading">
          Description
        </h2>
        <label className="visually-hidden" htmlFor="task-description">
          Description
        </label>
        <textarea
          id="task-description"
          className="textarea narr__textarea"
          rows={8}
          value={draft}
          onChange={(event) => setDraft(event.target.value)}
          onKeyDown={(event) => {
            if (event.key === 'Escape') {
              event.stopPropagation()
              setDraft(task.description ?? '')
              setEditing(false)
            }
          }}
        />
        <div className="field__actions">
          <Button variant="primary" disabled={patch.isPending} onClick={save}>
            {patch.isPending ? 'Saving…' : 'Save'}
          </Button>
          <Button
            variant="subtle"
            onClick={() => {
              setDraft(task.description ?? '')
              setEditing(false)
            }}
          >
            Cancel
          </Button>
        </div>
        {patch.isError ? <ErrorNotice error={patch.error} /> : null}
      </section>
    )
  }

  const text = task.description ?? ''

  return (
    <section className="narr__section dsec" aria-labelledby="desc-heading">
      <div className="narr__sectionhead">
        <h2 id="desc-heading" className="narr__heading">
          Description
        </h2>
        {mayEdit ? (
          <Button variant="subtle" className="narr__edit" onClick={() => setEditing(true)}>
            {text === '' ? 'Add' : 'Edit'}
          </Button>
        ) : null}
      </div>
      {text === '' ? (
        <p className="narr__none">No description.</p>
      ) : (
        /* `pre-wrap`, not a markdown renderer: the server stores plain text and
           there is no decision on record about a markup dialect. Rendering one
           anyway would be inventing it, and would mangle every description that
           legitimately contains an underscore. */
        <p className="narr__body">{text}</p>
      )}
    </section>
  )
}
