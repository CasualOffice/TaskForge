/**
 * `/settings/tags` — the shared vocabulary.
 *
 * # Why tags are authored here and not typed onto a task
 *
 * Authoring the vocabulary is `tag.manage`; applying an existing tag is
 * `task.update`. A picker that created whatever was typed would make every typo
 * a permanent term in a set everyone shares, and would do it under the wrong
 * permission. So the task surface applies by id, and new terms start here.
 *
 * # Scope is a real choice, not a detail
 *
 * A tag with no project belongs to the workspace and every project may use it.
 * One scoped to a project is refused elsewhere with a `422`, which a user would
 * meet as a picker offering an option the save rejects. The form says which is
 * which before the choice is made.
 */
import { Button, Input, Select } from '@schnsrw/design-system'
import { useState, type ReactElement } from 'react'
import { useQuery } from '@tanstack/react-query'

import { keys } from '../api/keys'
import { listProjects } from '../api/projects'
import { createTag, listTags } from '../api/tags'
import { useAuthority } from '../shell/permissions'
import { useWorkspaceId } from '../shell/session'
import { ErrorNotice } from '../shell/notice'
import { Field, Form, Loading, NeedsPermission, Section, useWrite, WriteError } from './parts'

export function TagsSettings(): ReactElement {
  const workspaceId = useWorkspaceId()
  const authority = useAuthority()
  const mayManage = authority.can('tag.manage')

  const [name, setName] = useState('')
  const [projectId, setProjectId] = useState('')
  const [color, setColor] = useState('')

  const tags = useQuery({
    queryKey: keys.tags(workspaceId),
    queryFn: ({ signal }) => listTags(workspaceId, undefined, signal),
    enabled: workspaceId !== '',
  })
  const projects = useQuery({
    queryKey: keys.projects(workspaceId),
    queryFn: ({ signal }) => listProjects(workspaceId, signal),
    enabled: workspaceId !== '',
  })

  const create = useWrite({
    run: () =>
      createTag(workspaceId, {
        name: name.trim(),
        project_id: projectId === '' ? undefined : projectId,
        color: color.trim() === '' ? undefined : color.trim(),
      }),
    announce: (tag) => `Created the tag ${tag.name}.`,
    invalidates: [keys.tags(workspaceId), keys.tags(workspaceId, projectId)],
    onDone: () => {
      setName('')
      setColor('')
    },
  })

  const projectName = new Map((projects.data?.data ?? []).map((p) => [p.id, p.name]))
  const rows = tags.data?.data ?? []

  return (
    <Section
      title="Tags"
      description="The vocabulary everyone shares. Applying a tag to a task is a different permission from authoring one."
    >
      {mayManage ? (
        <Form onSubmit={() => create.submit(undefined)}>
          <Field label="Name" id="tag-name">
            <Input
              full
              id="tag-name"
              value={name}
              maxLength={200}
              onChange={(event) => setName(event.target.value)}
            />
          </Field>
          <Field
            label="Available in"
            id="tag-scope"
            hint="A project-scoped tag is refused on tasks anywhere else."
          >
            <Select
              full
              id="tag-scope"
              value={projectId}
              onChange={(event) => setProjectId(event.target.value)}
            >
              <option value="">Every project in this workspace</option>
              {(projects.data?.data ?? []).map((project) => (
                <option key={project.id} value={project.id}>
                  Only {project.name}
                </option>
              ))}
            </Select>
          </Field>
          <Field
            label="Colour"
            id="tag-color"
            hint="A hint only. Every surface renders the name too, so colour is never the sole carrier of meaning."
          >
            <Input
              full
              id="tag-color"
              value={color}
              placeholder="#7a5cff"
              onChange={(event) => setColor(event.target.value)}
            />
          </Field>
          <WriteError error={create.error} />
          <Button variant="primary" type="submit" disabled={create.pending || name.trim() === ''}>
            {create.pending ? 'Creating…' : 'Create tag'}
          </Button>
        </Form>
      ) : (
        <NeedsPermission permission="tag.manage" />
      )}

      {tags.isPending ? <Loading label="Loading tags" /> : null}
      {tags.error ? <ErrorNotice error={tags.error} /> : null}
      {!tags.isPending && rows.length === 0 ? <p className="empty">No tags yet.</p> : null}
      <ul className="settings__rows">
        {rows.map((tag) => (
          <li className="settings__row" key={tag.id}>
            <span className="settings__row-main">
              {tag.color === null ? null : (
                <span className="dot" style={{ background: tag.color }} aria-hidden="true" />
              )}
              {tag.name}
            </span>
            <span className="settings__row-meta">
              {tag.project_id === null
                ? 'every project'
                : `only ${projectName.get(tag.project_id) ?? 'one project'}`}
            </span>
          </li>
        ))}
      </ul>
      <p className="field__hint">
        Renaming and deleting a tag are not served by the API yet, so they are not offered here.
      </p>
    </Section>
  )
}
