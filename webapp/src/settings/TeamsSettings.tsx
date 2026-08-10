/**
 * `/settings/teams` — the groups a grant can name.
 *
 * # A team is a principal, not a folder
 *
 * `docs/04` lets a grant name a team, and everyone in it inherits. That is the
 * only thing a team is for. So this screen answers one question — who does a
 * grant on this team reach? — and the membership list *is* that answer rather
 * than an organizational nicety.
 *
 * # Why membership expands in place
 *
 * A team's members are a second request per team, and issuing all of them on
 * load would be one request per row for a list most people scan without opening.
 * Expanding fetches; collapsing keeps what it fetched, so opening the same team
 * twice is free.
 */
import { Badge, Button, Input, Select } from '@schnsrw/design-system'
import { useState, type ReactElement } from 'react'
import { useQuery } from '@tanstack/react-query'

import {
  addTeamMember,
  createTeam,
  listMembers,
  listTeamMembers,
  listTeams,
  removeTeamMember,
  type Team,
} from '../api/admin'
import { keys } from '../api/keys'
import { useAuthority } from '../shell/permissions'
import { useWorkspaceId } from '../shell/session'
import { ErrorNotice } from '../shell/notice'
import { Field, Form, Loading, NeedsPermission, useWrite, WriteError, PageHead } from './parts'

export function TeamsSettings(): ReactElement {
  const workspaceId = useWorkspaceId()
  const authority = useAuthority()
  // Teams are workspace configuration; `workspace.manage` is what the server
  // requires to change them, and there is no narrower team permission.
  const mayManage = authority.can('workspace.manage')
  const [name, setName] = useState('')

  const teams = useQuery({
    queryKey: keys.teams(workspaceId),
    queryFn: ({ signal }) => listTeams(workspaceId, signal),
    enabled: workspaceId !== '',
  })

  const create = useWrite({
    run: () => createTeam(workspaceId, name.trim()),
    announce: (team) => `Created the team ${team.name}.`,
    invalidates: [keys.teams(workspaceId)],
    onDone: () => setName(''),
  })

  const rows = teams.data?.data ?? []

  return (
    <PageHead
      title="Teams"
      description="A team is something a role can be granted to. Everyone in it inherits that grant."
    >
      {mayManage ? (
        <Form onSubmit={() => create.submit(undefined)}>
          <Field label="New team" id="team-name">
            <Input
              full
              id="team-name"
              value={name}
              maxLength={200}
              onChange={(event) => setName(event.target.value)}
            />
          </Field>
          <WriteError error={create.error} />
          <Button variant="primary" type="submit" disabled={create.pending || name.trim() === ''}>
            {create.pending ? 'Creating…' : 'Create team'}
          </Button>
        </Form>
      ) : (
        <NeedsPermission permission="workspace.manage" />
      )}

      {teams.isPending ? <Loading label="Loading teams" /> : null}
      {teams.error ? <ErrorNotice error={teams.error} /> : null}
      {!teams.isPending && rows.length === 0 ? (
        <p className="empty">
          No teams yet. Without one, every grant has to name a person individually.
        </p>
      ) : null}

      <ul className="settings__rows">
        {rows.map((team) => (
          <TeamRow key={team.id} team={team} mayManage={mayManage} />
        ))}
      </ul>
    </PageHead>
  )
}

function TeamRow({ team, mayManage }: { team: Team; mayManage: boolean }): ReactElement {
  const [open, setOpen] = useState(false)
  return (
    <li className="settings__row settings__row--stacked">
      <div className="settings__row-head">
        <span className="settings__row-main">{team.name}</span>
        <Button variant="subtle" aria-expanded={open} onClick={() => setOpen(!open)}>
          {open ? 'Hide members' : 'Members'}
        </Button>
      </div>
      {open ? <TeamMembers teamId={team.id} teamName={team.name} mayManage={mayManage} /> : null}
    </li>
  )
}

function TeamMembers({
  teamId,
  teamName,
  mayManage,
}: {
  teamId: string
  teamName: string
  mayManage: boolean
}): ReactElement {
  const workspaceId = useWorkspaceId()
  const [chosen, setChosen] = useState('')

  const members = useQuery({
    queryKey: keys.teamMembers(workspaceId, teamId),
    queryFn: ({ signal }) => listTeamMembers(workspaceId, teamId, signal),
  })
  // The workspace directory, to choose from. Everyone in a team must already be
  // a workspace member — the server refuses otherwise (`TF-VAL-0007`), because
  // `team_membership` carries no tenant of its own.
  const everyone = useQuery({
    queryKey: keys.members(workspaceId),
    queryFn: ({ signal }) => listMembers(workspaceId, undefined, signal),
  })

  const add = useWrite({
    run: (userId: string) => addTeamMember(workspaceId, teamId, userId),
    announce: () => `Added to ${teamName}.`,
    invalidates: [keys.teamMembers(workspaceId, teamId)],
    onDone: () => setChosen(''),
  })
  const remove = useWrite({
    run: (userId: string) => removeTeamMember(workspaceId, teamId, userId),
    announce: () => `Removed from ${teamName}.`,
    invalidates: [keys.teamMembers(workspaceId, teamId)],
  })

  const inTeam = members.data?.data ?? []
  const inTeamIds = new Set(inTeam.map((member) => member.user_id))
  const addable = (everyone.data?.data ?? []).filter((member) => !inTeamIds.has(member.user_id))

  return (
    <div className="settings__nested">
      {members.isPending ? <Loading rows={2} label={`Loading ${teamName}`} /> : null}
      {members.error ? <ErrorNotice error={members.error} /> : null}
      {!members.isPending && inTeam.length === 0 ? (
        <p className="field__hint">Nobody is in this team, so a grant on it reaches nobody.</p>
      ) : null}

      <ul className="settings__rows">
        {inTeam.map((member) => (
          <li className="settings__row" key={member.user_id}>
            <span className="settings__row-main">
              {member.display_name}
              {member.member_type === 'GUEST' ? <Badge tone="neutral">guest</Badge> : null}
            </span>
            <span className="settings__row-meta">{member.email ?? 'address removed'}</span>
            {mayManage ? (
              <Button
                variant="subtle"
                onClick={() => remove.submit(member.user_id)}
                disabled={remove.pending}
              >
                Remove
              </Button>
            ) : null}
          </li>
        ))}
      </ul>

      <WriteError error={add.error ?? remove.error} />

      {mayManage && addable.length > 0 ? (
        <Form onSubmit={() => (chosen === '' ? undefined : add.submit(chosen))}>
          <Field label={`Add someone to ${teamName}`} id={`team-add-${teamId}`}>
            <Select
              full
              id={`team-add-${teamId}`}
              value={chosen}
              onChange={(event) => setChosen(event.target.value)}
            >
              <option value="">Choose a workspace member…</option>
              {addable.map((member) => (
                <option key={member.user_id} value={member.user_id}>
                  {member.display_name} · {member.email ?? 'address removed'}
                </option>
              ))}
            </Select>
          </Field>
          <Button variant="secondary" type="submit" disabled={add.pending || chosen === ''}>
            {add.pending ? 'Adding…' : 'Add to team'}
          </Button>
        </Form>
      ) : null}
    </div>
  )
}
