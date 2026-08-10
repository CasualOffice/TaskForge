/**
 * `/settings/workspace` — the tenant's own identity.
 *
 * # Why the slug is shown and not editable
 *
 * It is in every URL anyone has pasted into a ticket or a chat. The server
 * offers no rename for it, and a disabled field that says why is more useful
 * than no field at all: the question "can I change this?" is the one a person
 * opens this page to answer.
 *
 * # Why the rename is conditional
 *
 * Two admins can hold this page open at once, which is the lost update
 * `docs/24` exists for. The version comes from the `ETag` — `WorkspaceBody`
 * carries no `version` field, so the header is the only source.
 */
import { Badge, Button, Input } from '@schnsrw/design-system'
import { useEffect, useState, type ReactElement } from 'react'
import { useQuery } from '@tanstack/react-query'

import { requestWithVersion } from '../api/http'
import { keys } from '../api/keys'
import { renameWorkspace, type Workspace } from '../api/admin'
import { useAuthority } from '../shell/permissions'
import { useSession, useWorkspaceId } from '../shell/session'
import { ErrorNotice } from '../shell/notice'
import {
  Field,
  Form,
  Loading,
  NeedsPermission,
  Section,
  useWrite,
  WriteError,
  PageHead,
} from './parts'

export function WorkspaceSettings(): ReactElement {
  const workspaceId = useWorkspaceId()
  const authority = useAuthority()

  const workspace = useQuery({
    queryKey: keys.workspaceSettings(workspaceId),
    queryFn: ({ signal }) =>
      requestWithVersion<Workspace>(`/api/v1/workspaces/${workspaceId}`, { workspaceId, signal }),
    enabled: workspaceId !== '',
  })

  if (workspace.isPending || authority.loading) {
    return <Loading rows={3} label="Loading workspace settings" />
  }
  if (workspace.error) return <ErrorNotice error={workspace.error} />
  if (workspace.data === undefined) return <p className="empty">This workspace is unavailable.</p>

  return (
    <Identity
      workspace={workspace.data.data}
      version={workspace.data.version}
      mayManage={authority.can('workspace.manage')}
    />
  )
}

function Identity({
  workspace,
  version,
  mayManage,
}: {
  workspace: Workspace
  version: number | undefined
  mayManage: boolean
}): ReactElement {
  const workspaceId = useWorkspaceId()
  const [name, setName] = useState(workspace.name)

  // The server's value wins when it changes underneath — a rename by the other
  // admin holding this page open should be visible, not silently overwritten by
  // whatever this tab had in its input.
  useEffect(() => setName(workspace.name), [workspace.name])

  const rename = useWrite({
    run: (next: string) => {
      if (version === undefined) {
        // Refusing here rather than sending: without a version the request is
        // unconditional, the server answers 428, and the user reads a
        // precondition error for a form they filled in correctly.
        throw new Error(
          'This workspace was read without a version, so it cannot be renamed safely.',
        )
      }
      return renameWorkspace(workspaceId, version, next)
    },
    announce: (updated) => `Renamed to ${updated.name}.`,
    invalidates: [keys.workspaceSettings(workspaceId), keys.workspaces()],
  })

  return (
    <>
      <PageHead
        title="Workspace"
        description="What this workspace is called, and the identifier everything else refers to it by."
      >
        {mayManage ? null : <NeedsPermission permission="workspace.manage" />}
        <Form onSubmit={() => rename.submit(name.trim())}>
          <Field label="Name" id="ws-name">
            <Input
              full
              id="ws-name"
              value={name}
              maxLength={200}
              disabled={!mayManage}
              onChange={(event) => setName(event.target.value)}
            />
          </Field>
          <Field
            label="Identifier"
            id="ws-slug"
            hint="Permanent. It is in every link anyone has shared, so there is no rename for it."
          >
            <Input full id="ws-slug" value={workspace.slug} readOnly disabled />
          </Field>
          <WriteError error={rename.error} />
          <Button
            variant="primary"
            type="submit"
            disabled={
              !mayManage || rename.pending || name.trim() === '' || name.trim() === workspace.name
            }
          >
            {rename.pending ? 'Renaming…' : 'Rename workspace'}
          </Button>
        </Form>
      </PageHead>
      <Switcher />
    </>
  )
}

/**
 * Which workspace this browser is looking at.
 *
 * Here as well as in the header because this is the page someone opens when
 * they are in the wrong one — and a settings screen that could only configure
 * a workspace you had already selected elsewhere is a scavenger hunt.
 */
function Switcher(): ReactElement {
  const { workspaces, workspace, chooseWorkspace } = useSession()
  if (workspaces.length < 2) return <></>
  return (
    <Section
      title="Other workspaces"
      description="You belong to more than one. Everything on these settings pages applies to the selected one."
    >
      <ul className="settings__rows">
        {workspaces.map((candidate) => (
          <li className="settings__row" key={candidate.id}>
            <span className="settings__row-main">{candidate.name}</span>
            <span className="settings__row-meta">{candidate.slug}</span>
            {candidate.id === workspace?.id ? (
              <Badge tone="accent">selected</Badge>
            ) : (
              <Button variant="subtle" onClick={() => chooseWorkspace(candidate.id)}>
                Switch to it
              </Button>
            )}
          </li>
        ))}
      </ul>
    </Section>
  )
}
