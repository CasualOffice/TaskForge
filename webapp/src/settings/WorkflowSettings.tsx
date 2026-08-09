/**
 * `/settings/workflow` — the statuses, and the moves between them.
 *
 * # A status without its state is a label
 *
 * `docs/23`: every status maps onto one of five permanent states, and that
 * mapping is what every filter, report and "is this done?" question reads. The
 * name is what a workspace calls it. So the state is chosen when a status is
 * authored, and it is shown on every row — a workflow whose "Done" column is
 * `ACTIVE` looks correct and reports wrongly, and this is the only screen where
 * that is visible.
 *
 * # Deleting a status always moves its tasks
 *
 * A status holding tasks cannot vanish: `DELETE …?migrate_to=` moves every task
 * on it in the same transaction, attributed to the admin who asked. The control
 * therefore asks where they go *before* it deletes, and shows how many will
 * move — the count comes from the server, so it is the real number rather than
 * whatever this page loaded earlier.
 *
 * # Why the transition list is a matrix and not a list of arrows
 *
 * "Can a task go from Blocked to Done?" is the question people bring here, and a
 * flat list of edges makes them scan for a pair. Rows are sources, columns are
 * destinations, and the absent cells are as informative as the present ones.
 */
import { Badge, Button, Input, Select } from '@schnsrw/design-system'
import { useState, type ReactElement } from 'react'
import { useQuery } from '@tanstack/react-query'

import { keys } from '../api/keys'
import {
  createStatus,
  createTransition,
  deleteStatus,
  deleteTransition,
  listStatusUsage,
  readWorkflowForEditing,
  reorderStatuses,
  TASK_STATES,
  type TaskState,
  type Workflow,
  type WorkflowStatus,
} from '../api/workflow'
import { listProjects } from '../api/projects'
import { useAuthority } from '../shell/permissions'
import { useWorkspaceId } from '../shell/session'
import { ErrorNotice } from '../shell/notice'
import { Field, Form, Loading, NeedsPermission, Section, useWrite, WriteError } from './parts'

export function WorkflowSettings(): ReactElement {
  const workspaceId = useWorkspaceId()
  const authority = useAuthority()
  const mayManage = authority.can('project.workflow.manage')

  // The workflow is reached through a project: `GET /workflows/{id}` needs an
  // id, and a project is where one is published. The first project's workflow
  // is the default in every workspace this product creates.
  const projects = useQuery({
    queryKey: keys.projects(workspaceId),
    queryFn: ({ signal }) => listProjects(workspaceId, signal),
    enabled: workspaceId !== '',
  })
  const workflowId = projects.data?.data[0]?.workflow_id

  const workflow = useQuery({
    queryKey: keys.workflow(workspaceId, workflowId ?? ''),
    queryFn: ({ signal }) => readWorkflowForEditing(workspaceId, workflowId as string, signal),
    enabled: workspaceId !== '' && workflowId !== undefined,
  })

  const usage = useQuery({
    queryKey: [...keys.workflow(workspaceId, workflowId ?? ''), 'usage'],
    queryFn: ({ signal }) => listStatusUsage(workspaceId, workflowId as string, signal),
    enabled: workspaceId !== '' && workflowId !== undefined,
  })

  if (projects.isPending) return <Loading rows={5} label="Loading the workflow" />
  if (projects.error) return <ErrorNotice error={projects.error} />
  if (workflowId === undefined) {
    return (
      <p className="empty">
        There are no projects yet, and a workflow belongs to one. Create a project first.
      </p>
    )
  }
  if (workflow.isPending) return <Loading rows={5} label="Loading the workflow" />
  if (workflow.error) return <ErrorNotice error={workflow.error} />
  if (workflow.data === undefined) return <p className="empty">This workflow is unavailable.</p>

  const counts = new Map((usage.data?.data ?? []).map((row) => [row.id, row.task_count]))

  return (
    <>
      <Statuses
        workflow={workflow.data.data}
        version={workflow.data.version}
        counts={counts}
        mayManage={mayManage}
      />
      <Transitions
        workflow={workflow.data.data}
        version={workflow.data.version}
        mayManage={mayManage}
      />
    </>
  )
}

function Statuses({
  workflow,
  version,
  counts,
  mayManage,
}: {
  workflow: Workflow
  version: number | undefined
  counts: ReadonlyMap<string, number>
  mayManage: boolean
}): ReactElement {
  const workspaceId = useWorkspaceId()
  const [name, setName] = useState('')
  const [state, setState] = useState<TaskState>('ACTIVE')
  const invalidates = [keys.workflow(workspaceId, workflow.id), keys.taskLists(workspaceId)]

  const add = useWrite({
    run: () => createStatus(workspaceId, workflow.id, version ?? 0, { name: name.trim(), state }),
    announce: () => `Added ${name.trim()}.`,
    invalidates,
    onDone: () => setName(''),
  })

  const move = useWrite({
    run: (order: readonly string[]) =>
      reorderStatuses(workspaceId, workflow.id, version ?? 0, order),
    announce: () => 'Reordered.',
    invalidates,
  })

  const ordered = [...workflow.statuses].sort((a, b) => a.position - b.position)

  const swap = (index: number, by: number): void => {
    const next = [...ordered]
    const target = next[index + by]
    const current = next[index]
    if (target === undefined || current === undefined) return
    next[index + by] = current
    next[index] = target
    // The whole order, not a move: a partial list is a workflow with a hole.
    move.submit(next.map((status) => status.id))
  }

  return (
    <Section
      title="Statuses"
      description="The columns of a board, in order. Each maps onto one of five permanent states, which is what reports read."
    >
      {mayManage ? null : <NeedsPermission permission="project.workflow.manage" />}
      <WriteError error={move.error} />
      <ul className="settings__rows">
        {ordered.map((status, index) => (
          <li className="settings__row" key={status.id}>
            <span className="settings__row-main">
              {status.name}
              {status.is_initial ? <Badge tone="info">new tasks start here</Badge> : null}
            </span>
            <span className="settings__row-meta">
              {status.state.toLowerCase()} · {counts.get(status.id) ?? 0} tasks
            </span>
            {mayManage ? (
              <>
                <Button
                  variant="subtle"
                  aria-label={`Move ${status.name} earlier`}
                  disabled={index === 0 || move.pending}
                  onClick={() => swap(index, -1)}
                >
                  ↑
                </Button>
                <Button
                  variant="subtle"
                  aria-label={`Move ${status.name} later`}
                  disabled={index === ordered.length - 1 || move.pending}
                  onClick={() => swap(index, 1)}
                >
                  ↓
                </Button>
                <DeleteStatus
                  workflow={workflow}
                  version={version}
                  status={status}
                  holding={counts.get(status.id) ?? 0}
                />
              </>
            ) : null}
          </li>
        ))}
      </ul>

      {mayManage ? (
        <Form onSubmit={() => add.submit(undefined)}>
          <Field label="New status" id="status-name">
            <Input
              full
              id="status-name"
              value={name}
              maxLength={200}
              onChange={(event) => setName(event.target.value)}
            />
          </Field>
          <Field
            label="Permanent state"
            id="status-state"
            hint="What this status means to reports and filters. It cannot be inferred from the name."
          >
            <Select
              full
              id="status-state"
              value={state}
              onChange={(event) => setState(event.target.value as TaskState)}
            >
              {TASK_STATES.map((option) => (
                <option key={option} value={option}>
                  {option.toLowerCase()}
                </option>
              ))}
            </Select>
          </Field>
          <WriteError error={add.error} />
          <Button variant="primary" type="submit" disabled={add.pending || name.trim() === ''}>
            {add.pending ? 'Adding…' : 'Add status'}
          </Button>
        </Form>
      ) : null}
    </Section>
  )
}

/**
 * Delete, having first asked where the tasks go.
 *
 * Two steps rather than a confirm dialog: the destination is a *decision*, not a
 * confirmation, and a dialog that asked "are you sure?" would still have to ask
 * the real question afterwards.
 */
function DeleteStatus({
  workflow,
  version,
  status,
  holding,
}: {
  workflow: Workflow
  version: number | undefined
  status: WorkflowStatus
  holding: number
}): ReactElement {
  const workspaceId = useWorkspaceId()
  const [asking, setAsking] = useState(false)
  const [destination, setDestination] = useState('')

  const remove = useWrite({
    run: () => deleteStatus(workspaceId, workflow.id, status.id, version ?? 0, destination),
    announce: () => `Deleted ${status.name}.`,
    invalidates: [keys.workflow(workspaceId, workflow.id), keys.taskLists(workspaceId)],
    onDone: () => setAsking(false),
  })

  const elsewhere = workflow.statuses.filter((candidate) => candidate.id !== status.id)

  if (!asking) {
    return (
      <Button variant="subtle" onClick={() => setAsking(true)} disabled={elsewhere.length === 0}>
        Delete
      </Button>
    )
  }

  return (
    <Form className="settings__inline-form" onSubmit={() => remove.submit(undefined)}>
      <Field
        label={
          holding === 0
            ? `Delete ${status.name}`
            : `Move ${holding} ${holding === 1 ? 'task' : 'tasks'} from ${status.name} to`
        }
        id={`migrate-${status.id}`}
      >
        <Select
          full
          id={`migrate-${status.id}`}
          value={destination}
          onChange={(event) => setDestination(event.target.value)}
        >
          <option value="">Choose a status…</option>
          {elsewhere.map((candidate) => (
            <option key={candidate.id} value={candidate.id}>
              {candidate.name}
            </option>
          ))}
        </Select>
      </Field>
      <WriteError error={remove.error} />
      <Button variant="secondary" type="submit" disabled={remove.pending || destination === ''}>
        {remove.pending ? 'Deleting…' : 'Delete and move'}
      </Button>
      <Button variant="subtle" onClick={() => setAsking(false)}>
        Cancel
      </Button>
    </Form>
  )
}

function Transitions({
  workflow,
  version,
  mayManage,
}: {
  workflow: Workflow
  version: number | undefined
  mayManage: boolean
}): ReactElement {
  const workspaceId = useWorkspaceId()
  const invalidates = [keys.workflow(workspaceId, workflow.id)]

  const add = useWrite({
    run: (edge: { from: string | null; to: string }) =>
      createTransition(workspaceId, workflow.id, version ?? 0, edge),
    announce: () => 'Move allowed.',
    invalidates,
  })
  const remove = useWrite({
    run: (id: string) => deleteTransition(workspaceId, workflow.id, id, version ?? 0),
    announce: () => 'Move removed.',
    invalidates,
  })

  const ordered = [...workflow.statuses].sort((a, b) => a.position - b.position)
  const edge = (from: string, to: string): string | undefined =>
    workflow.transitions.find((t) => t.from === from && t.to === to)?.id
  /** `from: null` — the wildcard `docs/23` uses for "from anywhere". */
  const anywhere = workflow.transitions.filter((t) => t.from === null)

  return (
    <Section
      title="Allowed moves"
      description="A row is where a task is; a column is where it may go. An empty cell is a move nobody can make."
    >
      <WriteError error={add.error ?? remove.error} />
      <div className="settings__matrix-scroll">
        <table className="settings__matrix">
          <caption className="visually-hidden">
            Allowed transitions. Rows are the current status, columns the destination.
          </caption>
          <thead>
            <tr>
              <th scope="col">From ↓ / To →</th>
              {ordered.map((status) => (
                <th scope="col" key={status.id}>
                  {status.name}
                </th>
              ))}
            </tr>
          </thead>
          <tbody>
            {ordered.map((from) => (
              <tr key={from.id}>
                <th scope="row">{from.name}</th>
                {ordered.map((to) => {
                  const existing = from.id === to.id ? undefined : edge(from.id, to.id)
                  return (
                    <td key={to.id}>
                      {from.id === to.id ? (
                        <span aria-hidden="true">·</span>
                      ) : (
                        <Button
                          variant="subtle"
                          aria-pressed={existing !== undefined}
                          aria-label={`${from.name} to ${to.name}${existing === undefined ? ': not allowed' : ': allowed'}`}
                          disabled={!mayManage || add.pending || remove.pending}
                          onClick={() =>
                            existing === undefined
                              ? add.submit({ from: from.id, to: to.id })
                              : remove.submit(existing)
                          }
                        >
                          {existing === undefined ? '—' : '✓'}
                        </Button>
                      )}
                    </td>
                  )
                })}
              </tr>
            ))}
          </tbody>
        </table>
      </div>
      {anywhere.length > 0 ? (
        <p className="field__hint">
          {anywhere.length === 1 ? 'One move is' : `${anywhere.length} moves are`} allowed from any
          status:{' '}
          {anywhere
            .map((t) => ordered.find((s) => s.id === t.to)?.name ?? 'a deleted status')
            .join(', ')}
          . Those are not in the grid, and editing them is not built here yet.
        </p>
      ) : null}
      {mayManage ? null : <NeedsPermission permission="project.workflow.manage" />}
    </Section>
  )
}
