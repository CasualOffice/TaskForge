/**
 * Where this task sits in the tree: one parent above, its children below.
 *
 * # Why the parent is here and not in the metadata column
 *
 * "What is this part of?" is a navigational question, and the answer is a link
 * someone follows. The metadata column is short by construction — one line per
 * field, nothing behind a disclosure — and a parent belongs beside the children
 * it shares a tree with, not filed as a property.
 *
 * # The counts come from the server, and that matters
 *
 * `done` and `total` are counted across **all** children, not the page below.
 * A progress line computed from a truncated list would read "3 of 5 done" on a
 * task with forty children, which is worse than no progress line at all.
 */
import type { ReactElement } from 'react'
import { useQuery } from '@tanstack/react-query'
import { Link } from '@tanstack/react-router'

import { keys } from '../api/keys'
import { readSubtasks } from '../api/relations'
import type { Task } from '../api/tasks'
import { ErrorNotice } from '../shell/notice'
import { useWorkspaceId } from '../shell/session'

export function Subtasks({ taskId }: { taskId: string }): ReactElement | null {
  const workspaceId = useWorkspaceId()
  const tree = useQuery({
    queryKey: keys.subtasks(workspaceId, taskId),
    queryFn: ({ signal }) => readSubtasks(workspaceId, taskId, signal),
    enabled: workspaceId !== '',
  })

  if (tree.error) return <ErrorNotice error={tree.error} />
  const data = tree.data
  if (data === undefined) return null

  // A leaf with no parent has nothing to say here. Rendering an empty "Subtasks"
  // heading on every task would put a permanent blank section on most of them.
  if (data.parent === null && data.total === 0) return null

  return (
    <section className="sub" aria-labelledby="subtasks-heading">
      <h2 id="subtasks-heading" className="narr__heading">
        {data.parent === null ? 'Subtasks' : 'Part of'}
      </h2>

      {data.parent === null ? null : (
        <p className="sub__parent">
          <Link to="/tasks/$taskId" params={{ taskId: data.parent.id }}>
            <span className="key">{data.parent.key}</span> {data.parent.title}
          </Link>
        </p>
      )}

      {data.total === 0 ? null : (
        <>
          <p className="sub__progress">
            {data.done} of {data.total} done
          </p>
          <ul className="sub__list">
            {data.data.map((child) => (
              <ChildRow key={child.id} task={child} />
            ))}
          </ul>
          {data.truncated ? (
            <p className="field__hint">
              Showing the first {data.data.length} of {data.total}.
            </p>
          ) : null}
        </>
      )}
    </section>
  )
}

function ChildRow({ task }: { task: Task }): ReactElement {
  const done = task.state === 'COMPLETED' || task.state === 'CANCELED'
  return (
    <li className="sub__row">
      {/* The state is carried by the strike-through *and* by the text, because
          `docs/42` forbids colour or styling as the sole carrier of meaning. */}
      <Link to="/tasks/$taskId" params={{ taskId: task.id }} className={done ? 'sub__done' : ''}>
        <span className="key">{task.key}</span> {task.title}
      </Link>
      <span className="sub__state">{task.state.toLowerCase()}</span>
    </li>
  )
}
