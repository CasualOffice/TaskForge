/**
 * Blockers and blocked work — two lists, never one.
 *
 * `design/LAYOUT-AND-INTERACTION-GUIDELINES.md` §12. A blocker gates this task's
 * transitions (ADR-019); a task this one blocks does not gate anything here. One
 * merged "related" list would put a thing that stops you beside a thing that
 * does not, and the reader would have to work out which is which from an arrow.
 *
 * # Why a refused cycle is rendered in full
 *
 * The server names the loop — `ONB-4 → API-2 → ONB-4` — because "invalid
 * dependency" tells a user nothing they can act on. That message is the whole
 * value of the refusal, so it is shown as it arrives rather than replaced with a
 * generic sentence.
 *
 * # Why `restricted` rows are drawn rather than dropped
 *
 * `docs/03`: a blocking task the viewer cannot see shows as restricted, never as
 * its title. Dropping the row would show a task as blocked by nothing, which
 * reads as "you may move this" — the opposite of true.
 */
import { useState, type ReactElement } from 'react'
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query'
import { Link } from '@tanstack/react-router'

import { keys } from '../api/keys'
import { PERMISSIONS } from '../api/permissions'
import {
  addDependency,
  removeDependency,
  type Relation,
  type Relations as RelationSet,
} from '../api/relations'
import { readRelations } from '../api/relations'
import { asApiError } from '../api/problem'
import { listTasks } from '../api/tasks'
import { useAnnounce } from '../shell/announce'
import { ErrorNotice } from '../shell/notice'
import type { Authority } from '../shell/permissions'
import { useWorkspaceId } from '../shell/session'

export function RelationsPanel({
  taskId,
  taskKey,
  authority,
}: {
  taskId: string
  taskKey: string
  authority: Authority
}): ReactElement {
  const workspaceId = useWorkspaceId()
  const relations = useQuery({
    queryKey: keys.relations(workspaceId, taskId),
    queryFn: ({ signal }) => readRelations(workspaceId, taskId, signal),
    enabled: workspaceId !== '',
  })

  const set = relations.data
  const nothing =
    set !== undefined && set.blocked_by.length === 0 && set.blocks.length === 0

  return (
    <section className="rel" aria-labelledby="relations-heading">
      <h2 id="relations-heading" className="narr__heading">
        Blockers
      </h2>

      {relations.error ? <ErrorNotice error={relations.error} /> : null}

      {/* Both lists are always headed, even when empty, because "nothing is
          blocking this" is an answer people come here for. */}
      <RelationList
        title="Blocked by"
        empty="Nothing is blocking this."
        rows={set?.blocked_by ?? []}
        taskId={taskId}
        may={authority.can(PERMISSIONS.taskUpdate)}
      />
      <RelationList
        title="Blocks"
        empty="This is not holding anything up."
        rows={set?.blocks ?? []}
        taskId={taskId}
        may={authority.can(PERMISSIONS.taskUpdate)}
      />

      {authority.can(PERMISSIONS.taskUpdate) ? (
        <AddRelation taskId={taskId} taskKey={taskKey} />
      ) : nothing ? null : null}
    </section>
  )
}

function RelationList({
  title,
  empty,
  rows,
  taskId,
  may,
}: {
  title: string
  empty: string
  rows: readonly Relation[]
  taskId: string
  may: boolean
}): ReactElement {
  return (
    <div className="rel__group">
      <h3 className="rel__title">{title}</h3>
      {rows.length === 0 ? (
        <p className="rel__empty">{empty}</p>
      ) : (
        <ul className="rel__list">
          {rows.map((row) => (
            <RelationRow key={row.id ?? `restricted-${title}`} row={row} taskId={taskId} may={may} />
          ))}
        </ul>
      )}
    </div>
  )
}

function RelationRow({
  row,
  taskId,
  may,
}: {
  row: Relation
  taskId: string
  may: boolean
}): ReactElement {
  const workspaceId = useWorkspaceId()
  const client = useQueryClient()
  const announce = useAnnounce()

  const remove = useMutation({
    mutationFn: (otherId: string) => removeDependency(workspaceId, taskId, otherId),
    onSuccess: (next: RelationSet) => {
      // The response is the panel's new state, so it is written straight into
      // the cache rather than triggering a second read of what we just changed.
      client.setQueryData(keys.relations(workspaceId, taskId), next)
      // The blocked flag on the task itself may have changed with it.
      void client.invalidateQueries({ queryKey: keys.taskLists(workspaceId) })
      announce('Dependency removed')
    },
    onError: (error) => announce(asApiError(error).sentence, 'error'),
  })

  if (row.restricted || row.id === null) {
    // Named as a fact, not as an error. The person cannot see the task and can
    // still see that something is in the way, which is the point.
    return (
      <li className="rel__row rel__row--restricted">
        <span>A task in a project you cannot see</span>
      </li>
    )
  }

  const done = row.state === 'COMPLETED' || row.state === 'CANCELED'

  return (
    <li className="rel__row">
      {/* Struck through when resolved: a blocker that is already Done is not
          holding anything up, and it should not read like one. */}
      <Link to="/tasks/$taskId" params={{ taskId: row.id }} className={done ? 'rel__done' : ''}>
        <span className="key">{row.key}</span> {row.title}
      </Link>
      {may ? (
        <button
          type="button"
          className="button button--quiet"
          aria-label={`Remove the dependency on ${row.key}`}
          disabled={remove.isPending}
          onClick={() => remove.mutate(row.id as string)}
        >
          Remove
        </button>
      ) : null}
    </li>
  )
}

/**
 * Add an edge, naming the other task by its key.
 *
 * # Why a key and not an id
 *
 * `WR-125` is what people have in front of them, in a ticket or a message.
 * Nobody has a UUID. The endpoint takes an id, so the key is resolved first
 * through the filter grammar — `key` is in its closed field set — and the
 * refusal for a key that matches nothing is written here rather than left to a
 * `422` about a field the user never saw.
 *
 * A picker over every task would be a 2M-row dropdown, and the search that would
 * make one usable is the palette's job, not this panel's.
 */
function AddRelation({ taskId, taskKey }: { taskId: string; taskKey: string }): ReactElement {
  const workspaceId = useWorkspaceId()
  const client = useQueryClient()
  const announce = useAnnounce()
  const [open, setOpen] = useState(false)
  const [direction, setDirection] = useState<'blocked_by' | 'blocks'>('blocked_by')
  const [other, setOther] = useState('')

  const add = useMutation({
    mutationFn: async (typed: string) => {
      const id = await resolve(workspaceId, typed)
      return addDependency(
        workspaceId,
        taskId,
        direction === 'blocks' ? { blocks: id } : { blocked_by: id },
      )
    },
    onSuccess: (next: RelationSet) => {
      client.setQueryData(keys.relations(workspaceId, taskId), next)
      void client.invalidateQueries({ queryKey: keys.taskLists(workspaceId) })
      announce('Dependency added')
      setOther('')
      setOpen(false)
    },
    onError: (error) => announce(asApiError(error).sentence, 'error'),
  })

  if (!open) {
    return (
      <button type="button" className="button button--quiet" onClick={() => setOpen(true)}>
        Add a blocker
      </button>
    )
  }

  return (
    <form
      className="rel__form"
      onSubmit={(event) => {
        event.preventDefault()
        const trimmed = other.trim()
        if (trimmed !== '') add.mutate(trimmed)
      }}
    >
      <label className="field__label" htmlFor="rel-direction">
        Direction
      </label>
      <select
        id="rel-direction"
        className="select"
        value={direction}
        onChange={(event) => setDirection(event.target.value as 'blocked_by' | 'blocks')}
      >
        <option value="blocked_by">Something blocks {taskKey}</option>
        <option value="blocks">{taskKey} blocks something</option>
      </select>

      <label className="field__label" htmlFor="rel-other">
        The other task, by key
      </label>
      <input
        id="rel-other"
        className="input"
        value={other}
        placeholder={`${taskKey.split('-')[0] ?? 'WR'}-125`}
        onChange={(event) => setOther(event.target.value)}
      />

      {add.error ? <ErrorNotice error={add.error} /> : null}

      <button type="submit" className="button" disabled={add.isPending || other.trim() === ''}>
        {add.isPending ? 'Adding…' : 'Add'}
      </button>
      <button type="button" className="button button--quiet" onClick={() => setOpen(false)}>
        Cancel
      </button>
    </form>
  )
}

/**
 * A task key to its id.
 *
 * `key` is in the filter grammar's closed field set, so this is the same
 * endpoint every list uses rather than a lookup route invented for one panel.
 * A key that matches nothing throws here, which puts the sentence beside the
 * field the user typed into — better than a `422` naming an id they never saw.
 */
async function resolve(workspaceId: string, typed: string): Promise<string> {
  const key = typed.trim().toUpperCase()
  const found = await listTasks(workspaceId, { filter: { key }, limit: 1 })
  const first = found.data[0]
  if (first === undefined) throw new Error(`No task called ${key}.`)
  return first.id
}
