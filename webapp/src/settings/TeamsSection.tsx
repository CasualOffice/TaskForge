/**
 * Teams — which exist because a grant can be assigned to one.
 *
 * # Why a team is worth explaining on screen
 *
 * `docs/04` §Vocabulary lists three kinds of principal a role can be granted to:
 * a user, a **team**, and a service account. That is the entire reason teams
 * exist in this product — they are not a directory feature or a chat concept.
 * Granting "Project Manager on Platform" to a team of six is one row that stays
 * correct as people join and leave; six user grants is six rows that go stale
 * silently. `workspaces/mod.rs` says it in one line: "teams, which exist because
 * a grant can be assigned to one." An administrator who does not know that will
 * make teams and wonder what they did.
 *
 * # What this section cannot do, and why it says so instead of hiding it
 *
 * **A team's roster cannot be read.** `server.rs` registers
 * `POST /api/v1/teams/{id}/members` and `DELETE .../members/{user_id}` and
 * **no `GET`**. So members can be added and removed, and the result cannot be
 * displayed. That is the honest state, and it is stated where the roster would
 * be — a section that silently omitted the list would look finished and be
 * wrong, and one that hid the add control because the list is missing would
 * remove a capability that works.
 *
 * Adding is therefore idempotent in effect and blind in presentation: the
 * confirmation says what was sent, not what the team now contains, because the
 * second is not knowable.
 */
import { useState, type ReactElement } from 'react'
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query'

import { keys } from '../api/keys'
import { addTeamMember, createTeam, listTeams, removeTeamMember, type Team } from '../api/admin'
import { listMembers } from '../api/workspaces'
import { ErrorNotice, GapNotice } from '../shell/notice'
import { EmptyState, Skeleton } from '../shell/EmptyState'
import { useAnnounce } from '../shell/announce'
import { Field, Panel } from './Field'
import { OpenQuestion } from './OpenQuestion'
import { formatDate } from './WorkspaceSection'
import type { SectionProps } from './sections'

const MAX_NAME = 200

function teamsKey(workspaceId: string): readonly unknown[] {
  return [...keys.workspace(workspaceId), 'teams']
}

export function TeamsSection({ workspaceId }: SectionProps): ReactElement {
  const announce = useAnnounce()
  const client = useQueryClient()

  const teams = useQuery({
    queryKey: teamsKey(workspaceId),
    queryFn: ({ signal }) => listTeams(workspaceId, signal),
    staleTime: 60_000,
  })

  const members = useQuery({
    queryKey: keys.members(workspaceId),
    queryFn: ({ signal }) => listMembers(workspaceId, signal),
    staleTime: 60_000,
  })

  const rows = teams.data?.data ?? []

  return (
    <div className="set-sections">
      <Panel
        title="Teams"
        description="A team is a principal a role can be granted to. Granting a role to a team once keeps working as people join and leave it; granting it to each person separately does not."
      >
        {teams.isPending ? (
          <Skeleton rows={3} label="Loading teams" />
        ) : teams.error !== null ? (
          <ErrorNotice error={teams.error} />
        ) : rows.length === 0 ? (
          <EmptyState
            title="No teams yet."
            detail="Create one below, then grant it a role instead of granting each person the same role separately."
          />
        ) : (
          <ul className="set-teams">
            {rows.map((team) => (
              <TeamCard
                key={team.id}
                team={team}
                workspaceId={workspaceId}
                members={members.data?.data ?? []}
                onChanged={announce}
              />
            ))}
          </ul>
        )}
      </Panel>

      <Panel title="New team">
        <CreateTeamForm
          workspaceId={workspaceId}
          onCreated={(name) => {
            announce(`Team ${name} created.`)
            void client.invalidateQueries({ queryKey: teamsKey(workspaceId) })
          }}
        />
      </Panel>

      <OpenQuestion id="D-054">
        Creating a team and changing its membership is offered to every member, because that is
        what the server enforces — no permission governs team management yet.
      </OpenQuestion>
    </div>
  )
}

function TeamCard({
  team,
  workspaceId,
  members,
  onChanged,
}: {
  team: Team
  workspaceId: string
  members: readonly { user_id: string; display_name: string }[]
  onChanged: (message: string) => void
}): ReactElement {
  const [chosen, setChosen] = useState('')

  const add = useMutation({
    mutationFn: (userId: string) => addTeamMember(workspaceId, team.id, userId),
    onSuccess: (_result, userId) => {
      onChanged(`${nameOf(members, userId)} added to ${team.name}.`)
      setChosen('')
    },
  })

  const remove = useMutation({
    mutationFn: (userId: string) => removeTeamMember(workspaceId, team.id, userId),
    onSuccess: (_result, userId) => {
      onChanged(`${nameOf(members, userId)} removed from ${team.name}.`)
      setChosen('')
    },
  })

  const busy = add.isPending || remove.isPending
  const error = add.error ?? remove.error

  return (
    <li className="set-team">
      <div className="set-team__head">
        <h4 className="set-team__name">{team.name}</h4>
        <span className="set-team__meta">created {formatDate(team.created_at)}</span>
      </div>

      {/* The API serves POST and DELETE on a team's members but no GET, so
          membership can be changed and not read back; adding someone already in
          the team is harmless. That is the engineering shape of it — the reader
          gets the consequence in one line, per `shell/notice.tsx`. */}
      <GapNotice what="Who is in this team cannot be shown yet." />

      <div className="set-team__controls">
        <Field label={`Workspace member`} hint="Only members of this workspace can be in its teams.">
          {(wiring) => (
            <select
              {...wiring}
              className="select"
              value={chosen}
              onChange={(event) => setChosen(event.target.value)}
            >
              <option value="">Choose a person…</option>
              {members.map((member) => (
                <option key={member.user_id} value={member.user_id}>
                  {member.display_name}
                </option>
              ))}
            </select>
          )}
        </Field>

        <div className="set-team__buttons">
          <button
            type="button"
            className="button"
            disabled={chosen === '' || busy}
            onClick={() => add.mutate(chosen)}
          >
            {add.isPending ? 'Adding…' : 'Add to team'}
          </button>
          {/* Removal sits apart from adding (LAYOUT §6) but is not behind a
              confirm: it revokes nothing by itself — the team's grants are
              separate rows — and it is undone by pressing Add. */}
          <button
            type="button"
            className="button button--quiet set-team__remove"
            disabled={chosen === '' || busy}
            onClick={() => remove.mutate(chosen)}
          >
            {remove.isPending ? 'Removing…' : 'Remove from team'}
          </button>
        </div>
      </div>

      <p className="set-team__consequence">
        Whoever is in {team.name} holds every role granted to it, at the scope it was granted.
      </p>

      {error === null || error === undefined ? null : <ErrorNotice error={error} />}
    </li>
  )
}

function CreateTeamForm({
  workspaceId,
  onCreated,
}: {
  workspaceId: string
  onCreated: (name: string) => void
}): ReactElement {
  const [name, setName] = useState('')
  const [touched, setTouched] = useState(false)

  const create = useMutation({
    mutationFn: () => createTeam(workspaceId, name.trim()),
    onSuccess: (team) => {
      setName('')
      setTouched(false)
      onCreated(team.name)
    },
  })

  const trimmed = name.trim()
  const problem = touched && trimmed === '' ? 'A team needs a name.' : undefined

  return (
    <form
      className="set-form"
      onSubmit={(event) => {
        event.preventDefault()
        setTouched(true)
        if (trimmed === '') return
        create.mutate()
      }}
    >
      <Field
        label="Team name"
        hint="Unique within the workspace. Name it after the group of people, not the project — a team can work on several."
        error={problem}
        required
      >
        {(wiring) => (
          <input
            {...wiring}
            className="input"
            type="text"
            value={name}
            maxLength={MAX_NAME}
            onChange={(event) => setName(event.target.value)}
          />
        )}
      </Field>
      <div className="set-form__actions">
        <button type="submit" className="button button--primary" disabled={create.isPending}>
          {create.isPending ? 'Creating…' : 'Create team'}
        </button>
      </div>
      {create.error === null || create.error === undefined ? null : (
        <ErrorNotice error={create.error} />
      )}
    </form>
  )
}

function nameOf(
  members: readonly { user_id: string; display_name: string }[],
  userId: string,
): string {
  return members.find((member) => member.user_id === userId)?.display_name ?? 'That person'
}
