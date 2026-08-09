/**
 * Who is working on a task.
 *
 * # The gap this file is honest about
 *
 * The assignee set is **write-only** in the API today. `POST` and `DELETE`
 * `/tasks/{id}/assignees` exist and both answer with the resulting set, but
 * `TaskView` carries no `assignees` field and there is no `GET` — so a freshly
 * loaded task cannot say who it is assigned to. Filtering by `assignee=@me`
 * works (that is how My Work is built), which makes the gap easy to miss:
 * the *query* knows, and the *representation* does not.
 *
 * This component therefore shows the set it has seen — the answer to the last
 * mutation in this session — and says so, rather than rendering an empty list
 * that reads as "nobody is assigned". A wrong fact stated confidently is worse
 * than a missing one stated plainly.
 *
 * Tracked as **C-008** in `docs/14-EXECUTION-TRACKER.md`.
 */
import { useState, type ReactElement } from 'react'
import { useMutation, useQuery } from '@tanstack/react-query'

import { keys } from '../api/keys'
import type { Task } from '../api/tasks'
import { assignTask, unassignTask } from '../api/tasks'
import { directory, listMembers } from '../api/workspaces'
import { useAnnounce } from '../shell/announce'
import { ErrorNotice, GapNotice } from '../shell/notice'
import { useWorkspaceId } from '../shell/session'

export function Assignees({ task }: { task: Task }): ReactElement {
  const workspaceId = useWorkspaceId()
  const announce = useAnnounce()
  const [known, setKnown] = useState<readonly string[] | undefined>(undefined)
  const [chosen, setChosen] = useState('')

  const members = useQuery({
    queryKey: keys.members(workspaceId),
    queryFn: ({ signal }) => listMembers(workspaceId, signal),
    enabled: workspaceId !== '',
    staleTime: 5 * 60_000,
  })

  const nameOf = directory(members.data?.data ?? [])

  const assign = useMutation({
    mutationFn: (userId: string) => assignTask(workspaceId, task.id, userId),
    onSuccess: (result, userId) => {
      setKnown(result.assignees)
      announce(`${nameOf(userId)} assigned to ${task.key}`)
    },
  })

  const unassign = useMutation({
    mutationFn: (userId: string) => unassignTask(workspaceId, task.id, userId),
    onSuccess: (_result, userId) => {
      setKnown((current) => (current ?? []).filter((id) => id !== userId))
      announce(`${nameOf(userId)} unassigned from ${task.key}`)
    },
  })

  return (
    <section className="drawer__section" aria-labelledby="assignees-heading">
      <h3 id="assignees-heading" className="drawer__section-title">
        Assignees
      </h3>

      {known === undefined ? (
        <GapNotice what="The assignee list is not readable yet." tracker="C-008">
          <span>
            <code>TaskView</code> carries no <code>assignees</code> field and there is no{' '}
            <code>GET /api/v1/tasks/&#123;id&#125;/assignees</code>. Assigning below works and
            reports the resulting set.
          </span>
        </GapNotice>
      ) : (
        <ul className="assignees">
          {known.length === 0 ? <li className="field__hint">Nobody.</li> : null}
          {known.map((id) => (
            <li key={id} className="assignees__row">
              <span>{nameOf(id)}</span>
              <button
                type="button"
                className="button button--quiet"
                disabled={unassign.isPending}
                onClick={() => unassign.mutate(id)}
              >
                Remove
              </button>
            </li>
          ))}
        </ul>
      )}

      <div className="field">
        <label className="field__label" htmlFor="assignee-picker">
          Assign someone
        </label>
        <div className="field__actions">
          <select
            id="assignee-picker"
            className="select"
            value={chosen}
            onChange={(event) => setChosen(event.target.value)}
          >
            <option value="">Choose a member…</option>
            {(members.data?.data ?? []).map((member) => (
              <option key={member.user_id} value={member.user_id}>
                {member.display_name}
              </option>
            ))}
          </select>
          <button
            type="button"
            className="button"
            disabled={chosen === '' || assign.isPending}
            onClick={() => assign.mutate(chosen)}
          >
            Assign
          </button>
        </div>
      </div>

      {assign.isError ? <ErrorNotice error={assign.error} /> : null}
      {unassign.isError ? <ErrorNotice error={unassign.error} /> : null}
    </section>
  )
}
