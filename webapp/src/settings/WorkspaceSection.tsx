/**
 * The workspace itself: what it is, renaming it, and leaving it.
 *
 * # The failure this module prevents
 *
 * A rename sent without `If-Match`, refused with `428 TF-CNC-0002`, and read by
 * the user as "the app is broken". `PATCH /workspaces/{id}` is a conditional
 * write, and `WorkspaceBody` does **not** serialize `version` the way every
 * other mutable aggregate does — the number exists only in the `ETag` header,
 * which the transport does not expose. So the form is built, and it renders the
 * blocked state with the reason instead of sending a request that cannot
 * succeed. A control that fails every time is worse than one that explains why
 * it is not offered.
 *
 * # What is gated, and the open question underneath it
 *
 * The rename control is gated on `workspace.manage`. **The server does not check
 * it** — `workspaces/lifecycle.rs` authorizes nothing beyond membership, because
 * **D-054** left "which permission governs workspace administration" open and
 * `AGENTS.md` forbids settling it in an implementation. So this gate is a
 * presentation choice, and it is stated on screen rather than hidden here: a
 * reader of the interface should not have to read the source to learn that the
 * lock on the door is painted on.
 */
import { useEffect, useState, type ReactElement } from 'react'
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query'

import { keys } from '../api/keys'
import { readWorkspace, removeMember, renameUnavailable, renameWorkspace } from '../api/admin'
import { listMembers } from '../api/workspaces'
import { asApiError } from '../api/problem'
import { useAnnounce } from '../shell/announce'
import { ErrorNotice, GapNotice } from '../shell/notice'
import { Skeleton } from '../shell/EmptyState'
import { useAuthority } from '../shell/permissions'
import { useSession } from '../shell/session'
import { Confirm } from './Confirm'
import { Danger, Field, Panel } from './Field'
import { NotPermitted, PermissionExplanation } from './PermissionExplanation'
import { OpenQuestion } from './OpenQuestion'
import type { SectionProps } from './sections'

const MAX_NAME = 200

export function WorkspaceSection({ workspaceId }: SectionProps): ReactElement {
  const client = useQueryClient()
  const announce = useAnnounce()
  const authority = useAuthority()
  const { actor, forget } = useSession()

  const workspace = useQuery({
    queryKey: [...keys.workspace(workspaceId), 'detail'],
    queryFn: ({ signal }) => readWorkspace(workspaceId, signal),
    staleTime: 60_000,
  })

  const members = useQuery({
    queryKey: keys.members(workspaceId),
    queryFn: ({ signal }) => listMembers(workspaceId, signal),
    staleTime: 60_000,
  })

  if (workspace.isPending) return <Skeleton rows={4} label="Loading workspace settings" />
  if (workspace.error !== null) return <ErrorNotice error={workspace.error} />

  const data = workspace.data
  const count = members.data?.data.length
  const more = members.data?.page.has_more === true

  return (
    <div className="set-sections">
      <Panel
        title="Workspace"
        description="The tenant every project, task and permission in this app belongs to."
      >
        <dl className="set-facts">
          <div className="set-facts__row">
            <dt>Name</dt>
            <dd>{data.name}</dd>
          </div>
          <div className="set-facts__row">
            <dt>Slug</dt>
            {/* Fixed at creation: no endpoint changes it, and it is the stable
                half of the identity. Shown as code because it is one. */}
            <dd>
              <code className="set-code">{data.slug}</code>
            </dd>
          </div>
          <div className="set-facts__row">
            <dt>Created</dt>
            <dd>{formatDate(data.created_at)}</dd>
          </div>
          <div className="set-facts__row">
            <dt>Members</dt>
            <dd>
              {count === undefined ? '—' : more ? `${count}+` : String(count)}
            </dd>
          </div>
        </dl>
      </Panel>

      {authority.can('workspace.manage') ? (
        <RenameForm
          workspaceId={workspaceId}
          currentName={data.name}
          onRenamed={(name) => {
            announce(`Workspace renamed to ${name}.`)
            void client.invalidateQueries({ queryKey: keys.workspaces() })
            void client.invalidateQueries({ queryKey: [...keys.workspace(workspaceId), 'detail'] })
          }}
        />
      ) : (
        <Panel title="Rename">
          <NotPermitted what="rename this workspace" permission="workspace.manage">
            <PermissionExplanation workspaceId={workspaceId} permission="workspace.manage" />
          </NotPermitted>
        </Panel>
      )}

      <Danger title="Irreversible">
        <LeaveWorkspace
          workspaceId={workspaceId}
          workspaceName={data.name}
          actorId={actor?.actor_id}
          memberCount={count}
          onLeft={() => {
            announce(`You left ${data.name}.`)
            forget()
          }}
        />

        {/* The permission exists — `workspace.delete` is in the closed registry
            — and no route is registered for it, so there is nothing for a
            control to call. That is why the reader gets a sentence and not a
            button. The engineering half of it belongs here, not on screen. */}
        <GapNotice what="A workspace cannot be deleted from here yet." />
      </Danger>

      <OpenQuestion id="D-054">
        Renaming is offered on <code className="set-code">workspace.manage</code>, but the server
        checks only that you are a member. Which permission governs workspace administration is
        recorded as open in the execution tracker, so this interface takes a position and says so
        rather than settling it quietly.
      </OpenQuestion>
    </div>
  )
}

function RenameForm({
  workspaceId,
  currentName,
  onRenamed,
}: {
  workspaceId: string
  currentName: string
  onRenamed: (name: string) => void
}): ReactElement {
  const [name, setName] = useState(currentName)
  const blocked = renameUnavailable()

  // A rename by somebody else, arriving over a refetch, should move the field
  // the user has not touched — not sit there showing a stale name they would
  // then re-submit and lose the conflict on.
  useEffect(() => setName(currentName), [currentName])

  const rename = useMutation({
    // `version` is unreachable from a browser today, so this never runs. The
    // call is kept wired rather than deleted: the mutation, the validation and
    // the conflict handling are the parts that are finished, and the day the
    // representation carries its version this becomes a one-argument change.
    mutationFn: (next: string) => renameWorkspace(workspaceId, next, 0),
    onSuccess: (updated) => onRenamed(updated.name),
  })

  const trimmed = name.trim()
  const unchanged = trimmed === currentName
  const tooLong = trimmed.length > MAX_NAME
  const empty = trimmed === ''

  const problem = rename.error === null || rename.error === undefined
    ? empty && rename.isIdle === false
      ? 'A workspace needs a name.'
      : tooLong
        ? `A name is at most ${MAX_NAME} characters. This one is ${trimmed.length}.`
        : undefined
    : asApiError(rename.error).sentence

  return (
    <Panel title="Rename" description="The name everyone in the workspace sees. The slug does not change.">
      {blocked !== undefined ? (
        /* Closes when `WorkspaceBody` serializes `version` like every other
           mutable aggregate — one field, and the form below starts working
           unchanged. `blocked` is already a sentence a reader can act on. */
        <GapNotice what={blocked} />
      ) : (
        <form
          className="set-form"
          onSubmit={(event) => {
            event.preventDefault()
            if (empty || tooLong || unchanged) return
            rename.mutate(trimmed)
          }}
        >
          <Field label="Workspace name" error={problem} required>
            {(wiring) => (
              <input
                {...wiring}
                className="input"
                type="text"
                value={name}
                maxLength={MAX_NAME + 1}
                onChange={(event) => setName(event.target.value)}
              />
            )}
          </Field>
          <div className="set-form__actions">
            <button
              type="submit"
              className="button button--primary"
              disabled={rename.isPending || unchanged || empty || tooLong}
            >
              {rename.isPending ? 'Saving…' : 'Save name'}
            </button>
            {unchanged ? null : (
              <button type="button" className="button" onClick={() => setName(currentName)}>
                Reset
              </button>
            )}
          </div>
          {rename.error === null || rename.error === undefined ? null : (
            <ErrorNotice error={rename.error} />
          )}
        </form>
      )}
    </Panel>
  )
}

/**
 * Leaving is removing yourself, and it is the one destructive membership action
 * a person can always take about themselves.
 *
 * Refused when you are the last member — `422 TF-PRJ-0006`, decided under the
 * workspace row's write lock. That refusal is anticipated here rather than
 * discovered: a workspace with one member shows the reason **before** the
 * gesture, because a confirm dialog that leads to a refusal is two dead ends
 * instead of one.
 */
function LeaveWorkspace({
  workspaceId,
  workspaceName,
  actorId,
  memberCount,
  onLeft,
}: {
  workspaceId: string
  workspaceName: string
  actorId: string | undefined
  memberCount: number | undefined
  onLeft: () => void
}): ReactElement {
  const [asking, setAsking] = useState(false)

  const leave = useMutation({
    mutationFn: () => removeMember(workspaceId, actorId ?? ''),
    onSuccess: () => {
      setAsking(false)
      onLeft()
    },
  })

  const alone = memberCount === 1

  return (
    <div className="set-danger__row">
      <div className="set-danger__text">
        <p className="set-danger__label">Leave this workspace</p>
        <p className="set-danger__detail">
          You lose access to every project and task in {workspaceName}. Someone who remains has to
          invite you back.
        </p>
        {alone ? (
          <p className="set-danger__blocked">
            You are the only member. A workspace cannot lose its last one — nothing could see it
            afterwards, so nothing could add anyone back.
          </p>
        ) : null}
      </div>
      <button
        type="button"
        className="button button--danger"
        onClick={() => setAsking(true)}
        disabled={alone || actorId === undefined}
      >
        Leave
      </button>

      <Confirm
        open={asking}
        title={`Leave ${workspaceName}?`}
        confirmLabel="Leave"
        busy={leave.isPending}
        onCancel={() => setAsking(false)}
        onConfirm={() => leave.mutate()}
      >
        <p>
          Your membership is removed immediately. Tasks you reported and comments you wrote stay
          where they are.
        </p>
        {leave.error === null || leave.error === undefined ? null : (
          <ErrorNotice error={leave.error} />
        )}
      </Confirm>
    </div>
  )
}

/** A date a person reads, from the RFC 3339 the API sends. */
export function formatDate(iso: string): string {
  const at = new Date(iso)
  if (Number.isNaN(at.getTime())) return iso
  return at.toLocaleDateString(undefined, { year: 'numeric', month: 'short', day: 'numeric' })
}
