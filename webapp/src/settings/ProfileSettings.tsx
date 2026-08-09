/**
 * `/settings/profile` — the account, not the workspace.
 *
 * # Why the time zone matters more than it looks
 *
 * `@today`, `+7d` and every relative date in the filter grammar are resolved
 * against it (`docs/27`). Without one the server falls back to UTC, which is
 * wrong by up to a day at either end for most of the world: "due today" in
 * Auckland at 09:00 is yesterday in UTC. The field is a plain text input rather
 * than a picker because the browser already knows the answer — the control
 * offers it and the user confirms.
 *
 * # Why the password form warns before it is pressed
 *
 * Migration 0016: changing a password refuses every session older than the
 * change, **including the caller's other tabs**. Telling someone afterwards that
 * they have been signed out of their phone is a support ticket; telling them
 * before is a decision they made.
 */
import { useState, type ReactElement } from 'react'
import { useQuery } from '@tanstack/react-query'

import { keys } from '../api/keys'
import {
  changePassword,
  listSessions,
  readMe,
  revokeOtherSessions,
  revokeSession,
  updateMe,
  type LiveSession,
} from '../api/me'
import { ErrorNotice } from '../shell/notice'
import { Field, Form, Loading, Section, useWrite, WriteError } from './parts'

export function ProfileSettings(): ReactElement {
  const me = useQuery({ queryKey: keys.me(), queryFn: ({ signal }) => readMe(signal) })

  if (me.isPending) return <Loading rows={4} label="Loading your profile" />
  if (me.error) return <ErrorNotice error={me.error} />
  if (me.data === undefined) return <p className="empty">Your profile is unavailable.</p>

  return (
    <>
      <Identity
        displayName={me.data.display_name}
        timeZone={me.data.time_zone}
        email={me.data.email}
      />
      <Password />
      <Sessions />
    </>
  )
}

function Identity({
  displayName,
  timeZone,
  email,
}: {
  displayName: string
  timeZone: string | null
  email: string | null
}): ReactElement {
  const [name, setName] = useState(displayName)
  const [zone, setZone] = useState(timeZone ?? '')
  const detected = Intl.DateTimeFormat().resolvedOptions().timeZone

  const save = useWrite({
    run: () =>
      updateMe({
        display_name: name.trim(),
        // Empty clears it. `null` and `undefined` are different requests
        // (`docs/05`): one says "use UTC", the other says "I did not touch it".
        time_zone: zone.trim() === '' ? null : zone.trim(),
      }),
    announce: (updated) => `Saved. You are ${updated.display_name}.`,
    invalidates: [keys.me(), keys.session()],
  })

  const unchanged = name.trim() === displayName && zone.trim() === (timeZone ?? '')

  return (
    <Section
      title="Your profile"
      description={
        email === null
          ? 'This account has been anonymized and has no address.'
          : `Signed in as ${email}. The address cannot be changed here.`
      }
    >
      <Form onSubmit={() => save.submit(undefined)}>
        <Field label="Display name" id="profile-name">
          <input
            id="profile-name"
            className="input"
            value={name}
            maxLength={200}
            onChange={(event) => setName(event.target.value)}
          />
        </Field>
        <Field
          label="Time zone"
          id="profile-zone"
          hint="Relative dates — today, overdue, next week — are resolved in this zone. Empty means UTC."
        >
          <input
            id="profile-zone"
            className="input"
            value={zone}
            placeholder={detected}
            onChange={(event) => setZone(event.target.value)}
          />
        </Field>
        {zone.trim() === '' && detected !== '' ? (
          <p className="field__hint">
            Your browser reports <code>{detected}</code>.{' '}
            <button type="button" className="button button--quiet" onClick={() => setZone(detected)}>
              Use it
            </button>
          </p>
        ) : null}
        <WriteError error={save.error} />
        <button
          type="submit"
          className="button button--primary"
          disabled={save.pending || unchanged || name.trim() === ''}
        >
          {save.pending ? 'Saving…' : 'Save profile'}
        </button>
      </Form>
    </Section>
  )
}

function Password(): ReactElement {
  const [current, setCurrent] = useState('')
  const [next, setNext] = useState('')
  const [confirm, setConfirm] = useState('')

  const change = useWrite({
    run: () => changePassword(current, next),
    announce: () => 'Password changed. Every other session has been signed out.',
    invalidates: [keys.mySessions()],
    onDone: () => {
      setCurrent('')
      setNext('')
      setConfirm('')
    },
  })

  const mismatch = confirm !== '' && next !== confirm

  return (
    <Section
      title="Password"
      description="Changing it signs out every other session, including your other devices. This one stays."
    >
      <Form onSubmit={() => change.submit(undefined)}>
        <Field label="Current password" id="pw-current">
          <input
            id="pw-current"
            className="input"
            type="password"
            autoComplete="current-password"
            value={current}
            onChange={(event) => setCurrent(event.target.value)}
          />
        </Field>
        <Field label="New password" id="pw-new" hint="At least 12 characters.">
          <input
            id="pw-new"
            className="input"
            type="password"
            autoComplete="new-password"
            value={next}
            onChange={(event) => setNext(event.target.value)}
          />
        </Field>
        <Field label="New password again" id="pw-confirm">
          <input
            id="pw-confirm"
            className="input"
            type="password"
            autoComplete="new-password"
            value={confirm}
            aria-invalid={mismatch}
            onChange={(event) => setConfirm(event.target.value)}
          />
        </Field>
        {/* Checked here rather than sent: the server has no way to know the
            second field exists, so a mismatch would come back as a successful
            change to a password the user did not mean to type. */}
        {mismatch ? <p className="field__hint">The two new passwords do not match.</p> : null}
        <WriteError error={change.error} />
        <button
          type="submit"
          className="button"
          disabled={change.pending || current === '' || next === '' || next !== confirm}
        >
          {change.pending ? 'Changing…' : 'Change password'}
        </button>
      </Form>
    </Section>
  )
}

function Sessions(): ReactElement {
  const sessions = useQuery({
    queryKey: keys.mySessions(),
    queryFn: ({ signal }) => listSessions(signal),
  })

  const revoke = useWrite({
    run: (id: string) => revokeSession(id),
    announce: () => 'That session has been signed out.',
    invalidates: [keys.mySessions()],
  })
  const revokeRest = useWrite({
    run: () => revokeOtherSessions(),
    announce: () => 'Every other session has been signed out.',
    invalidates: [keys.mySessions()],
  })

  const rows = sessions.data?.data ?? []
  const others = rows.filter((row) => !row.current).length

  return (
    <Section
      title="Where you are signed in"
      description="Every live session on this account. Signing one out takes effect on its next request."
      actions={
        others === 0 ? undefined : (
          <button
            type="button"
            className="button"
            onClick={() => revokeRest.submit(undefined)}
            disabled={revokeRest.pending}
          >
            Sign out {others} other {others === 1 ? 'session' : 'sessions'}
          </button>
        )
      }
    >
      <WriteError error={revoke.error ?? revokeRest.error} />
      {sessions.isPending ? <Loading label="Loading sessions" /> : null}
      {sessions.error ? <ErrorNotice error={sessions.error} /> : null}
      {!sessions.isPending && rows.length === 0 ? (
        <p className="empty">No sessions — which should be impossible while you are reading this.</p>
      ) : null}
      <ul className="settings__rows">
        {rows.map((row) => (
          <SessionRow
            key={row.id}
            session={row}
            onRevoke={() => revoke.submit(row.id)}
            busy={revoke.pending}
          />
        ))}
      </ul>
    </Section>
  )
}

function SessionRow({
  session,
  onRevoke,
  busy,
}: {
  session: LiveSession
  onRevoke: () => void
  busy: boolean
}): ReactElement {
  return (
    <li className="settings__row">
      <span className="settings__row-main">
        {/* The agent string is what a person recognizes — "that is my phone" —
            and it is attacker-controlled text, so React escapes it and nothing
            here parses it into a friendly name it might be lying about. */}
        {session.user_agent ?? 'An unnamed client'}
        {session.current ? <span className="badge"> this session</span> : null}
      </span>
      <span className="settings__row-meta">
        {session.ip_address ?? 'no address recorded'} · last seen{' '}
        <time dateTime={session.last_seen_at}>{when(session.last_seen_at)}</time> · signed in with{' '}
        {session.auth_method.toLowerCase()}
      </span>
      {session.current ? null : (
        <button type="button" className="button button--quiet" onClick={onRevoke} disabled={busy}>
          Sign out
        </button>
      )}
    </li>
  )
}

/** The date, in the reader's own locale and zone — the browser knows both. */
function when(iso: string): string {
  const at = new Date(iso)
  if (Number.isNaN(at.getTime())) return iso
  return at.toLocaleString()
}
