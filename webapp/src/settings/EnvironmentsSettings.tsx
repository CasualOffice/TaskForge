/**
 * `/settings/environments` — a project's deployment pipeline.
 *
 * # Why this is settings and not a task control
 *
 * An environment is part of the permission model, not just a task field: a
 * grant can be narrowed to one, so authoring the list is authoring the scopes
 * people can be granted authority in. That is `project.update`, and it is an
 * administrative act rather than a field edit.
 *
 * # Why the page starts with a project picker
 *
 * Environments belong to a project — one project deploys through dev, qa,
 * staging; another goes straight to production — so there is no workspace-wide
 * pipeline to show. The picker is the first control rather than a filter bolted
 * on afterwards, because without a project this page has nothing to say.
 *
 * # Why order is a set operation
 *
 * Moving one environment changes the position of every environment it passed.
 * Sending one move at a time is a read-modify-write two people can hold at
 * once, and the losing write leaves a pipeline with two environments at the
 * same position. So the whole order goes in one request, and the server refuses
 * anything that is not exactly this project's environments.
 */
import { Button, Input, Select } from '@schnsrw/design-system'
import { useState, type ReactElement } from 'react'
import { useQuery } from '@tanstack/react-query'

import { keys } from '../api/keys'
import { listProjects } from '../api/projects'
import {
  createEnvironment,
  deleteEnvironment,
  listEnvironments,
  renameEnvironment,
  reorderEnvironments,
  type Environment,
} from '../api/environments'
import { useAuthority } from '../shell/permissions'
import { useWorkspaceId } from '../shell/session'
import { ErrorNotice } from '../shell/notice'
import { Field, Form, Loading, NeedsPermission, PageHead, useWrite, WriteError } from './parts'

export function EnvironmentsSettings(): ReactElement {
  const workspaceId = useWorkspaceId()
  const [projectId, setProjectId] = useState('')
  const authority = useAuthority(projectId === '' ? undefined : projectId)
  const mayManage = authority.can('project.update')

  const projects = useQuery({
    queryKey: keys.projects(workspaceId),
    queryFn: ({ signal }) => listProjects(workspaceId, signal),
    enabled: workspaceId !== '',
  })
  const available = projects.data?.data ?? []
  // One project and no choice to make is not a choice worth asking for.
  const chosen =
    projectId !== '' ? projectId : ((available.length === 1 ? available[0]?.id : '') ?? '')

  const environments = useQuery({
    queryKey: keys.environments(workspaceId, chosen),
    queryFn: ({ signal }) => listEnvironments(workspaceId, chosen, signal),
    enabled: workspaceId !== '' && chosen !== '',
  })
  const pipeline = [...(environments.data?.data ?? [])].sort((a, b) => a.position - b.position)

  return (
    <PageHead
      title="Environments"
      description="The stages a project deploys through, in the order it deploys them. A task records which one it has reached."
    >
      <Field label="Project" id="env-project" hint="Environments belong to a project.">
        <Select
          full
          id="env-project"
          value={chosen}
          onChange={(event) => setProjectId(event.target.value)}
        >
          <option value="">Choose a project…</option>
          {available.map((project) => (
            <option key={project.id} value={project.id}>
              {project.key} — {project.name}
            </option>
          ))}
        </Select>
      </Field>

      {chosen === '' ? null : !mayManage ? (
        <NeedsPermission permission="project.update" />
      ) : (
        <>
          {environments.error ? <ErrorNotice error={environments.error} /> : null}
          {environments.isPending ? <Loading rows={3} label="Loading environments" /> : null}
          {!environments.isPending && pipeline.length === 0 ? (
            <p className="empty">No environments yet. Add the first stage below.</p>
          ) : null}

          <Pipeline workspaceId={workspaceId} projectId={chosen} pipeline={pipeline} />
          <AddEnvironment workspaceId={workspaceId} projectId={chosen} />
        </>
      )}
    </PageHead>
  )
}

function Pipeline({
  workspaceId,
  projectId,
  pipeline,
}: {
  workspaceId: string
  projectId: string
  pipeline: readonly Environment[]
}): ReactElement | null {
  const reorder = useWrite({
    run: (order: readonly string[]) => reorderEnvironments(workspaceId, projectId, order),
    announce: () => 'Pipeline order saved.',
    invalidates: [keys.environments(workspaceId, projectId)],
  })

  if (pipeline.length === 0) return null

  /** The whole order with one entry moved — see the module note on why. */
  const moved = (from: number, to: number): string[] => {
    const ids = pipeline.map((environment) => environment.id)
    const [lifted] = ids.splice(from, 1)
    if (lifted !== undefined) ids.splice(to, 0, lifted)
    return ids
  }

  return (
    <>
      <WriteError error={reorder.error} />
      <ol className="settings__rows">
        {pipeline.map((environment, index) => (
          <li className="settings__row" key={environment.id}>
            <span className="settings__row-main">
              <span className="key">{index + 1}</span>
              <EnvironmentName
                workspaceId={workspaceId}
                projectId={projectId}
                environment={environment}
              />
            </span>
            {/* Buttons rather than drag: a pipeline is four or five items, and
                a keyboard user reordering by drag is a keyboard user who
                cannot. Both directions are the same one call. */}
            <Button
              variant="subtle"
              size="sm"
              icon="arrow_upward"
              aria-label={`Move ${environment.name} earlier`}
              disabled={index === 0 || reorder.pending}
              onClick={() => reorder.submit(moved(index, index - 1))}
            >
              Earlier
            </Button>
            <Button
              variant="subtle"
              size="sm"
              icon="arrow_downward"
              aria-label={`Move ${environment.name} later`}
              disabled={index === pipeline.length - 1 || reorder.pending}
              onClick={() => reorder.submit(moved(index, index + 1))}
            >
              Later
            </Button>
            <RemoveEnvironment
              workspaceId={workspaceId}
              projectId={projectId}
              environment={environment}
              others={pipeline.filter((other) => other.id !== environment.id)}
            />
          </li>
        ))}
      </ol>
    </>
  )
}

function EnvironmentName({
  workspaceId,
  projectId,
  environment,
}: {
  workspaceId: string
  projectId: string
  environment: Environment
}): ReactElement {
  const [editing, setEditing] = useState(false)
  const [name, setName] = useState(environment.name)

  const rename = useWrite({
    run: () => renameEnvironment(workspaceId, environment.id, name.trim()),
    announce: (next) => `Renamed to ${next.name}.`,
    invalidates: [keys.environments(workspaceId, projectId)],
    onDone: () => setEditing(false),
  })

  if (!editing) {
    return (
      <Button variant="subtle" onClick={() => setEditing(true)}>
        {environment.name}
      </Button>
    )
  }

  return (
    <Form onSubmit={() => rename.submit(undefined)}>
      <WriteError error={rename.error} />
      <Input
        aria-label={`Rename ${environment.name}`}
        value={name}
        onChange={(event) => setName(event.target.value)}
      />
      <Button variant="primary" type="submit" disabled={rename.pending || name.trim() === ''}>
        Save
      </Button>
      <Button variant="subtle" onClick={() => setEditing(false)}>
        Cancel
      </Button>
    </Form>
  )
}

function RemoveEnvironment({
  workspaceId,
  projectId,
  environment,
  others,
}: {
  workspaceId: string
  projectId: string
  environment: Environment
  others: readonly Environment[]
}): ReactElement {
  const [open, setOpen] = useState(false)
  const [target, setTarget] = useState('none')

  const remove = useWrite({
    run: () => deleteEnvironment(workspaceId, environment.id, target),
    announce: () => `${environment.name} removed.`,
    invalidates: [keys.environments(workspaceId, projectId)],
    onDone: () => setOpen(false),
  })

  if (!open) {
    return (
      <Button variant="subtle" size="sm" onClick={() => setOpen(true)}>
        Remove
      </Button>
    )
  }

  return (
    <Form onSubmit={() => remove.submit(undefined)}>
      <WriteError error={remove.error} />
      {/* The target is required and has no default. Tasks carrying an
          environment that vanishes are tasks whose history stops explaining
          them, so where they go is said out loud. */}
      <Field
        label={`Move tasks on ${environment.name} to`}
        id={`env-migrate-${environment.id}`}
        hint="Every task currently on this environment is moved."
      >
        <Select
          full
          id={`env-migrate-${environment.id}`}
          value={target}
          onChange={(event) => setTarget(event.target.value)}
        >
          <option value="none">No environment</option>
          {others.map((other) => (
            <option key={other.id} value={other.id}>
              {other.name}
            </option>
          ))}
        </Select>
      </Field>
      <Button variant="danger" type="submit" disabled={remove.pending}>
        Remove {environment.name}
      </Button>
      <Button variant="subtle" onClick={() => setOpen(false)}>
        Cancel
      </Button>
    </Form>
  )
}

function AddEnvironment({
  workspaceId,
  projectId,
}: {
  workspaceId: string
  projectId: string
}): ReactElement {
  const [name, setName] = useState('')
  const create = useWrite({
    run: () => createEnvironment(workspaceId, projectId, name.trim()),
    announce: (environment) => `Added ${environment.name}.`,
    invalidates: [keys.environments(workspaceId, projectId)],
    onDone: () => setName(''),
  })

  return (
    <Form onSubmit={() => create.submit(undefined)}>
      <WriteError error={create.error} />
      <Field
        label="Add a stage"
        id="env-new"
        hint="It joins the end of the pipeline. Reorder it from there."
      >
        <Input
          full
          id="env-new"
          value={name}
          placeholder="staging"
          onChange={(event) => setName(event.target.value)}
        />
      </Field>
      <Button variant="secondary" type="submit" disabled={create.pending || name.trim() === ''}>
        Add
      </Button>
    </Form>
  )
}
