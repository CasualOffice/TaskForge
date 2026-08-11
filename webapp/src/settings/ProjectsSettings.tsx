/**
 * `/settings/projects` — the container every task belongs to.
 *
 * # Why this page had to exist
 *
 * It did not, and the product could not be used without going around it. A
 * project is the thing tasks are created in, the thing the board is scoped to,
 * the thing a workflow belongs to and the thing permissions are granted on —
 * and there was no way to make one. Every project in a demo workspace arrived
 * through a seeding script, which meant the first thing a new workspace owner
 * could do was nothing.
 *
 * # Why the key is a one-time decision, said out loud
 *
 * ADR-007 freezes `key` at creation because it prefixes every task key the
 * project will ever mint — `WEB-142` ends up in commit messages, chat threads
 * and other people's tickets, and a key that could be edited would turn all of
 * those into dead references. The server answers `422` to any attempt to change
 * it. So the create form says "permanent" *before* the choice, and the edit
 * form does not offer the field at all: a control that exists and always fails
 * is worse than one that was never drawn.
 *
 * # Why visibility is a scale and not a checkbox
 *
 * `docs/22`: `PRIVATE` is the members of this project, `TEAM` widens to its
 * owning team, `WORKSPACE` to everyone in the workspace. Three points on one
 * axis, so they are one control listed in widening order, with the consequence
 * spelled out per option — "who can see this" is the question people actually
 * get wrong, and they get it wrong by reading a label without a consequence.
 */
import { Button, Input, Select } from '@schnsrw/design-system'
import { useState, type ReactElement } from 'react'
import { useQuery } from '@tanstack/react-query'

import { keys } from '../api/keys'
import {
  createProject,
  listProjects,
  updateProject,
  VISIBILITIES,
  type Project,
  type ProjectVisibility,
} from '../api/projects'
import { useAuthority } from '../shell/permissions'
import { useWorkspaceId } from '../shell/session'
import { ErrorNotice } from '../shell/notice'
import { Field, Form, Loading, NeedsPermission, useWrite, WriteError, PageHead } from './parts'

/** What each visibility actually means, in the reader's terms rather than the enum's. */
const VISIBILITY_COPY: Record<ProjectVisibility, { label: string; hint: string }> = {
  PRIVATE: { label: 'Private', hint: 'Only people granted access to this project.' },
  TEAM: { label: 'Team', hint: 'Everyone in the project’s owning team.' },
  WORKSPACE: { label: 'Workspace', hint: 'Everyone in this workspace.' },
}

/** ADR-007's format, checked here so the refusal arrives before the request. */
const KEY_SHAPE = /^[A-Z][A-Z0-9]{1,9}$/

export function ProjectsSettings(): ReactElement {
  const workspaceId = useWorkspaceId()
  const authority = useAuthority()
  const mayCreate = authority.can('project.create')

  const [key, setKey] = useState('')
  const [name, setName] = useState('')
  const [visibility, setVisibility] = useState<ProjectVisibility>('WORKSPACE')
  const [editing, setEditing] = useState<string | undefined>(undefined)

  const projects = useQuery({
    queryKey: keys.projects(workspaceId),
    queryFn: ({ signal }) => listProjects(workspaceId, signal),
    enabled: workspaceId !== '',
  })

  const create = useWrite({
    run: () =>
      createProject(workspaceId, { key: key.trim().toUpperCase(), name: name.trim(), visibility }),
    announce: (project) => `Created ${project.name}.`,
    invalidates: [keys.projects(workspaceId)],
    onDone: () => {
      setKey('')
      setName('')
    },
  })

  const rows = projects.data?.data ?? []
  // Checked here as well as by the server, because the server's refusal costs a
  // round trip and arrives after the form has been submitted — and because the
  // rule ("uppercase, 2–10, starts with a letter") is not guessable from a
  // rejected save.
  const keyIsWellFormed = KEY_SHAPE.test(key.trim().toUpperCase())
  const keyIsTaken = rows.some((p) => p.key === key.trim().toUpperCase())

  return (
    <PageHead
      title="Projects"
      description="Tasks live in a project. It owns the board, the workflow, and who can see the work."
    >
      {mayCreate ? (
        <Form onSubmit={() => create.submit(undefined)}>
          <Field
            label="Key"
            id="project-key"
            hint={
              key === ''
                ? 'Permanent. It prefixes every task key — WEB-142 — so it cannot be changed later.'
                : keyIsTaken
                  ? 'Another project already uses that key.'
                  : keyIsWellFormed
                    ? `Tasks will be numbered ${key.trim().toUpperCase()}-1, ${key.trim().toUpperCase()}-2, and so on. This cannot be changed later.`
                    : 'Two to ten characters: an uppercase letter, then uppercase letters or digits.'
            }
          >
            <Input
              full
              id="project-key"
              value={key}
              maxLength={10}
              // Uppercased as it is typed rather than on save: the hint below
              // previews the task keys this produces, and a preview that
              // disagreed with what was typed would read as a bug.
              onChange={(event) => setKey(event.target.value.toUpperCase())}
            />
          </Field>
          <Field label="Name" id="project-name">
            <Input
              full
              id="project-name"
              value={name}
              maxLength={200}
              onChange={(event) => setName(event.target.value)}
            />
          </Field>
          <Field
            label="Who can see it"
            id="project-visibility"
            hint={VISIBILITY_COPY[visibility].hint}
          >
            <Select
              full
              id="project-visibility"
              value={visibility}
              onChange={(event) => setVisibility(event.target.value as ProjectVisibility)}
            >
              {VISIBILITIES.map((option) => (
                <option key={option} value={option}>
                  {VISIBILITY_COPY[option].label}
                </option>
              ))}
            </Select>
          </Field>
          <WriteError error={create.error} />
          <Button
            variant="primary"
            type="submit"
            disabled={create.pending || name.trim() === '' || !keyIsWellFormed || keyIsTaken}
          >
            {create.pending ? 'Creating…' : 'Create project'}
          </Button>
        </Form>
      ) : (
        <NeedsPermission permission="project.create" />
      )}

      {projects.isPending ? <Loading label="Loading projects" /> : null}
      {projects.error ? <ErrorNotice error={projects.error} /> : null}
      {!projects.isPending && rows.length === 0 ? (
        <p className="empty">No projects yet. The first one is where tasks can start.</p>
      ) : null}

      <ul className="settings__rows">
        {rows.map((project) =>
          editing === project.id ? (
            <li className="settings__row settings__row--form" key={project.id}>
              <EditProject project={project} onDone={() => setEditing(undefined)} />
            </li>
          ) : (
            <li className="settings__row" key={project.id}>
              <span className="settings__row-main">
                <span className="projectkey">{project.key}</span>
                {project.name}
              </span>
              <span className="settings__row-meta">
                {VISIBILITY_COPY[project.visibility as ProjectVisibility]?.label ??
                  project.visibility}
              </span>
              <Button size="sm" onClick={() => setEditing(project.id)}>
                Edit
              </Button>
            </li>
          ),
        )}
      </ul>
    </PageHead>
  )
}

/**
 * Editing one project, in place.
 *
 * In place rather than on its own route because the change is two fields, and
 * a navigation for two fields loses the list you were comparing against — the
 * reason to rename a project is usually that it reads badly *beside the
 * others*.
 */
function EditProject({
  project,
  onDone,
}: {
  project: Project
  onDone: () => void
}): ReactElement {
  const workspaceId = useWorkspaceId()
  const authority = useAuthority()
  const mayUpdate = authority.can('project.update')

  const [name, setName] = useState(project.name)
  const [visibility, setVisibility] = useState<ProjectVisibility>(
    project.visibility as ProjectVisibility,
  )

  const save = useWrite({
    run: () => updateProject(workspaceId, project.id, project.version, { name: name.trim(), visibility }),
    announce: (updated) => `Saved ${updated.name}.`,
    invalidates: [keys.projects(workspaceId)],
    onDone,
  })

  if (!mayUpdate) return <NeedsPermission permission="project.update" />

  return (
    <Form onSubmit={() => save.submit(undefined)}>
      {/* Shown, not editable. The key is the one thing about a project that
          cannot change, and hiding it here would make the edit form look like
          the whole of what a project is. */}
      <Field label="Key" id={`edit-key-${project.id}`} hint="Permanent (ADR-007).">
        <Input full readOnly id={`edit-key-${project.id}`} value={project.key} />
      </Field>
      <Field label="Name" id={`edit-name-${project.id}`}>
        <Input
          full
          id={`edit-name-${project.id}`}
          value={name}
          maxLength={200}
          onChange={(event) => setName(event.target.value)}
        />
      </Field>
      <Field
        label="Who can see it"
        id={`edit-vis-${project.id}`}
        hint={VISIBILITY_COPY[visibility].hint}
      >
        <Select
          full
          id={`edit-vis-${project.id}`}
          value={visibility}
          onChange={(event) => setVisibility(event.target.value as ProjectVisibility)}
        >
          {VISIBILITIES.map((option) => (
            <option key={option} value={option}>
              {VISIBILITY_COPY[option].label}
            </option>
          ))}
        </Select>
      </Field>
      <WriteError error={save.error} />
      <div className="settings__actions">
        <Button variant="primary" type="submit" disabled={save.pending || name.trim() === ''}>
          {save.pending ? 'Saving…' : 'Save'}
        </Button>
        <Button type="button" onClick={onDone}>
          Cancel
        </Button>
      </div>
    </Form>
  )
}
