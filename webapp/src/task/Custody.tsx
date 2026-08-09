/**
 * Who holds this, where it has reached, and how it fared (`docs/45`).
 *
 * # The panel is the hand-off, not a history
 *
 * The lifecycle is a chain of custody, and the moment anyone opens a task is
 * usually the moment they are about to pass it on: a developer has fixed it and
 * is pushing to qa, QA has tested it and is recording a verdict, a lead has
 * realised it belongs to another team. So the *actions* are the panel, and the
 * log sits under them as evidence.
 *
 * # Why transfer says what it will do before it does it
 *
 * A transfer clears the assignees — that is the point, because the task has to
 * land in the receiving team's queue rather than staying attached to someone who
 * is finished with it. A control that did that silently would be the worst kind
 * of surprise, so the sentence is above the button and not in a toast after it.
 *
 * # Why a verdict does not move the task
 *
 * Recording FAIL leaves the status alone. What happens next is a transition the
 * person makes deliberately, and keeping them separate is what lets "failed
 * twice on qa, then passed" survive however many times the status has changed —
 * a status column only ever holds the latest value.
 */
import { CONTROL } from '../shell/controls'
import { Button, Input, Select } from '@schnsrw/design-system'
import { useState, type ReactElement } from 'react'
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query'

import { listTeams, type Team } from '../api/admin'
import {
  promote,
  readCustody,
  transferTeam,
  verify,
  type Custody as CustodyData,
  type Verdict,
} from '../api/custody'
import { listEnvironments, type Environment } from '../api/environments'
import { keys } from '../api/keys'
import { PERMISSIONS } from '../api/permissions'
import { asApiError } from '../api/problem'
import type { Task } from '../api/tasks'
import { directory, listMembers } from '../api/workspaces'
import { useAnnounce } from '../shell/announce'
import { ErrorNotice } from '../shell/notice'
import type { Authority } from '../shell/permissions'
import { useWorkspaceId } from '../shell/session'
import { formatRelative } from '../tasks/present'

export function Custody({
  task,
  authority,
}: {
  task: Task
  authority: Authority
}): ReactElement | null {
  const workspaceId = useWorkspaceId()

  const custody = useQuery({
    queryKey: keys.custody(workspaceId, task.id),
    queryFn: ({ signal }) => readCustody(workspaceId, task.id, signal),
    enabled: workspaceId !== '',
  })
  const environments = useQuery({
    queryKey: keys.environments(workspaceId, task.project_id),
    queryFn: ({ signal }) => listEnvironments(workspaceId, task.project_id, signal),
    enabled: workspaceId !== '',
    staleTime: 5 * 60_000,
  })
  const teams = useQuery({
    queryKey: keys.teams(workspaceId),
    queryFn: ({ signal }) => listTeams(workspaceId, signal),
    enabled: workspaceId !== '',
    staleTime: 5 * 60_000,
  })
  const members = useQuery({
    queryKey: keys.members(workspaceId),
    queryFn: ({ signal }) => listMembers(workspaceId, signal),
    enabled: workspaceId !== '',
    staleTime: 5 * 60_000,
  })

  if (custody.error) return <ErrorNotice error={custody.error} />
  const data = custody.data
  if (data === undefined) return null

  const envs = [...(environments.data?.data ?? [])].sort((a, b) => a.position - b.position)
  const teamList = teams.data?.data ?? []
  const nameOf = directory(members.data?.data ?? [])
  const envName = (id: string): string => envs.find((e) => e.id === id)?.name ?? 'an environment'
  const teamName = (id: string | null): string =>
    id === null ? 'nobody' : (teamList.find((t) => t.id === id)?.name ?? 'a team')

  return (
    <section className="cust" aria-labelledby="custody-heading">
      <h2 id="custody-heading" className="narr__heading">
        Custody
      </h2>

      <dl className="cust__now">
        <dt>Owned by</dt>
        <dd>
          {data.team_id === null ? (
            /* Not an error state. A task nobody has routed is the triage queue,
               and saying "unassigned" here would hide that it needs a decision. */
            <span className="cust__untriaged">Untriaged — nobody owns this yet</span>
          ) : (
            teamName(data.team_id)
          )}
        </dd>
        <dt>On</dt>
        <dd>
          {data.environment_id === null ? (
            <span className="meta2__unset">No environment</span>
          ) : (
            envName(data.environment_id)
          )}
        </dd>
      </dl>

      {authority.can(PERMISSIONS.taskUpdate) ? (
        <>
          <Handoff task={task} teams={teamList} current={data.team_id} />
          <Promote task={task} environments={envs} />
        </>
      ) : null}
      {authority.can(PERMISSIONS.taskTransition) ? (
        <Verify task={task} on={data.environment_id} envName={envName} />
      ) : null}

      <Trail data={data} envName={envName} teamName={teamName} nameOf={nameOf} />
    </section>
  )
}

/** Invalidates everything a custody write can change. */
function useCustodyWrite<TArgs, TResult>(
  taskId: string,
  run: (args: TArgs) => Promise<TResult>,
  said: (result: TResult) => string,
): {
  readonly submit: (args: TArgs) => void
  readonly pending: boolean
  readonly error: unknown
} {
  const workspaceId = useWorkspaceId()
  const client = useQueryClient()
  const announce = useAnnounce()
  const mutation = useMutation({
    mutationFn: run,
    onSuccess: (result) => {
      void client.invalidateQueries({ queryKey: keys.custody(workspaceId, taskId) })
      // The task itself may have moved: a transfer clears the assignees, a
      // promotion changes the environment. Both are on the task representation.
      void client.invalidateQueries({ queryKey: keys.task(workspaceId, taskId) })
      void client.invalidateQueries({ queryKey: keys.assignees(workspaceId, taskId) })
      void client.invalidateQueries({ queryKey: keys.taskLists(workspaceId) })
      announce(said(result))
    },
    onError: (error) => announce(asApiError(error).sentence, 'error'),
  })
  return { submit: mutation.mutate, pending: mutation.isPending, error: mutation.error }
}

function Handoff({
  task,
  teams,
  current,
}: {
  task: Task
  teams: readonly Team[]
  current: string | null
}): ReactElement {
  const workspaceId = useWorkspaceId()
  const [team, setTeam] = useState('')
  const [note, setNote] = useState('')

  const hand = useCustodyWrite(
    task.id,
    () => transferTeam(workspaceId, task.id, team, note),
    () => 'Handed over.',
  )

  const options = teams.filter((candidate) => candidate.id !== current)
  if (options.length === 0) return <></>

  return (
    <form
      className="cust__form"
      onSubmit={(event) => {
        event.preventDefault()
        if (team !== '') hand.submit(undefined)
      }}
    >
      <label className="field__label" htmlFor="cust-team">
        Hand over to
      </label>
      <Select
        width="auto"
        containerStyle={{ maxWidth: 260 }}
        style={{ height: CONTROL }}
        id="cust-team"
        value={team}
        onChange={(event) => setTeam(event.target.value)}
      >
        <option value="">Choose a team…</option>
        {options.map((candidate) => (
          <option key={candidate.id} value={candidate.id}>
            {candidate.name}
          </option>
        ))}
      </Select>
      <Input
        full
        value={note}
        placeholder="Why — the receiving team reads this"
        onChange={(event) => setNote(event.target.value)}
        aria-label="Why it is being handed over"
      />
      {/* Said before it is pressed, not after. The clearing is the point of the
          hand-off, and a person who did not expect it has lost their assignee. */}
      <p className="field__hint">
        This clears the current assignees, so it lands in that team&rsquo;s queue.
      </p>
      {hand.error ? <ErrorNotice error={hand.error} /> : null}
      <Button variant="secondary" type="submit" disabled={hand.pending || team === ''}>
        {hand.pending ? 'Handing over…' : 'Hand over'}
      </Button>
    </form>
  )
}

function Promote({
  task,
  environments,
}: {
  task: Task
  environments: readonly Environment[]
}): ReactElement {
  const workspaceId = useWorkspaceId()
  const [environment, setEnvironment] = useState('')

  const push = useCustodyWrite(
    task.id,
    () => promote(workspaceId, task.id, environment),
    () => 'Promoted.',
  )

  if (environments.length === 0) {
    return (
      <p className="field__hint">
        This project has no environments yet, so nothing can be promoted. Add them under the
        project&rsquo;s settings.
      </p>
    )
  }

  return (
    <form
      className="cust__form"
      onSubmit={(event) => {
        event.preventDefault()
        if (environment !== '') push.submit(undefined)
      }}
    >
      <label className="field__label" htmlFor="cust-env">
        It reached
      </label>
      {/* In deployment order, which is `position` — sorting by name would put
          production second. */}
      <Select
        width="auto"
        containerStyle={{ maxWidth: 260 }}
        style={{ height: CONTROL }}
        id="cust-env"
        value={environment}
        onChange={(event) => setEnvironment(event.target.value)}
      >
        <option value="">Choose an environment…</option>
        {environments.map((candidate) => (
          <option key={candidate.id} value={candidate.id}>
            {candidate.name}
          </option>
        ))}
      </Select>
      {push.error ? <ErrorNotice error={push.error} /> : null}
      <Button variant="secondary" type="submit" disabled={push.pending || environment === ''}>
        {push.pending ? 'Recording…' : 'Record promotion'}
      </Button>
    </form>
  )
}

function Verify({
  task,
  on,
  envName,
}: {
  task: Task
  on: string | null
  envName: (id: string) => string
}): ReactElement {
  const workspaceId = useWorkspaceId()
  const [note, setNote] = useState('')

  const record = useCustodyWrite(
    task.id,
    (verdict: Verdict) => verify(workspaceId, task.id, verdict, note),
    (result) => (result.verdict === 'PASS' ? 'Passed.' : 'Failed, and recorded.'),
  )

  if (on === null) {
    // A verdict against no environment is untraceable — "it works" is not a
    // result. Said here rather than left to a 422 after the press.
    return (
      <p className="field__hint">
        Record which environment this reached before verifying it — a verdict with no environment
        cannot be reproduced.
      </p>
    )
  }

  return (
    <div className="cust__form">
      <span className="field__label">Verify on {envName(on)}</span>
      <Input
        full
        value={note}
        placeholder="What you saw — evidence for a failure"
        onChange={(event) => setNote(event.target.value)}
        aria-label="Evidence"
      />
      {record.error ? <ErrorNotice error={record.error} /> : null}
      <div className="cust__verdicts">
        <Button variant="secondary" disabled={record.pending} onClick={() => record.submit('PASS')}>
          Passed
        </Button>
        <Button
          variant="danger"
          disabled={record.pending || note.trim() === ''}
          onClick={() => record.submit('FAIL')}
        >
          Failed
        </Button>
      </div>
      {/* A failure with no evidence is a message nobody can act on, so the
          control is disabled rather than the refusal being explained later. */}
      {note.trim() === '' ? (
        <p className="field__hint">A failure needs evidence before it can be recorded.</p>
      ) : null}
    </div>
  )
}

/**
 * The three logs, merged into one timeline.
 *
 * Merged and not stacked, because the question is "what happened to this", and
 * three separate lists make the reader interleave them by timestamp themselves —
 * which is the work the panel exists to do.
 */
function Trail({
  data,
  envName,
  teamName,
  nameOf,
}: {
  data: CustodyData
  envName: (id: string) => string
  teamName: (id: string | null) => string
  nameOf: (id: string) => string
}): ReactElement {
  // Defensive against a partial payload: a missing list makes this panel render
  // less, not the whole route disappear. The task surface is one route, and a
  // field this component happens to trust should not be able to take the
  // comments and the description down with it.
  const transfers = data.transfers ?? []
  const promotions = data.promotions ?? []
  const verifications = data.verifications ?? []

  const events = [
    ...transfers.map((t) => ({
      at: t.moved_at,
      who: t.moved_by,
      what:
        t.from_team_id === null
          ? `routed it to ${teamName(t.to_team_id)}`
          : `handed it from ${teamName(t.from_team_id)} to ${teamName(t.to_team_id)}`,
      note: t.note,
      tone: '',
    })),
    ...promotions.map((p) => ({
      at: p.promoted_at,
      who: p.promoted_by,
      what: `promoted it to ${envName(p.environment_id)}`,
      note: null,
      tone: '',
    })),
    ...verifications.map((v) => ({
      at: v.verified_at,
      who: v.verified_by,
      what: `${v.verdict === 'PASS' ? 'passed' : 'failed'} it on ${envName(v.environment_id)}`,
      note: v.note,
      tone: v.verdict === 'FAIL' ? 'cust__fail' : '',
    })),
  ].sort((a, b) => b.at.localeCompare(a.at))

  if (events.length === 0) {
    return <p className="cust__empty">Nothing has happened to this yet.</p>
  }

  const failures = verifications.filter((v) => v.verdict === 'FAIL').length
  const bounces = transfers.filter((t) => t.from_team_id !== null).length

  return (
    <>
      {/* The two numbers that expose a broken process, stated rather than left
          to be counted from the list below. */}
      {failures > 1 || bounces > 1 ? (
        <p className="cust__warn">
          {failures > 1 ? `Failed verification ${failures} times. ` : ''}
          {bounces > 1 ? `Handed between teams ${bounces} times.` : ''}
        </p>
      ) : null}
      <ol className="cust__trail">
        {events.map((event) => (
          <li key={`${event.at}-${event.what}`} className={event.tone}>
            <span className="cust__who">{nameOf(event.who)}</span> {event.what}{' '}
            <time dateTime={event.at}>{formatRelative(event.at)}</time>
            {event.note === null || event.note === '' ? null : (
              <span className="cust__note"> — {event.note}</span>
            )}
          </li>
        ))}
      </ol>
    </>
  )
}
