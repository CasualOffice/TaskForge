/** The three custody logs merged into one chronological trail. */
import type { ReactElement } from 'react'

import type { Custody } from '../api/custody'
import { formatRelative } from '../tasks/present'

export function CustodyTrail({
  data,
  envName,
  teamName,
  nameOf,
}: {
  data: Custody
  envName: (id: string) => string
  teamName: (id: string | null) => string
  nameOf: (id: string) => string
}): ReactElement {
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

  if (events.length === 0) return <p className="cust__empty">Nothing has happened to this yet.</p>

  const failures = verifications.filter((v) => v.verdict === 'FAIL').length
  const bounces = transfers.filter((t) => t.from_team_id !== null).length
  return (
    <>
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
