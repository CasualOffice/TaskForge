/**
 * `/settings/members` — who is in the workspace, and who has been asked.
 *
 * # Membership grants nothing, and the screen has to say so
 *
 * Migration 0003: "role_assignment is the ONLY source of authority in the
 * system." A member with no grant can sign in and see an empty product, which
 * looks like a broken invitation rather than a missing role. So each row shows
 * the roles that person actually holds, read from the grant list — and someone
 * holding none is called out rather than left looking configured.
 *
 * # Why invitations sit beside members and not on their own page
 *
 * They are the same question asked at two moments: who is here, and who is
 * arriving. Splitting them means an admin who has just invited someone has to
 * navigate to find out whether it worked.
 */
import { Badge, Button, Input, Select } from '@schnsrw/design-system'
import { useState, type ReactElement } from 'react'
import { useQuery } from '@tanstack/react-query'

import {
  createInvitation,
  listInvitations,
  listMembers,
  removeMember,
  revokeInvitation,
  type Invitation,
  type Member,
} from '../api/admin'
import { keys } from '../api/keys'
import { listAssignments, listRoles, type Assignment, type Role } from '../api/roles'
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

export function MembersSettings(): ReactElement {
  const workspaceId = useWorkspaceId()
  const authority = useAuthority()
  const mayManage = authority.can('workspace.manage')
  const maySeeGrants = authority.can('role.assign') || authority.can('role.manage')

  const members = useQuery({
    queryKey: keys.members(workspaceId),
    queryFn: ({ signal }) => listMembers(workspaceId, undefined, signal),
    enabled: workspaceId !== '',
  })

  // Only asked for when the caller may read it — the endpoint refuses otherwise,
  // and a 403 in the background would surface as a broken page rather than as
  // the absent column it actually is.
  const grants = useQuery({
    queryKey: keys.assignmentsFor(workspaceId, { scope: 'all' }),
    queryFn: ({ signal }) => listAssignments(workspaceId, {}, signal),
    enabled: workspaceId !== '' && maySeeGrants,
  })
  const roles = useQuery({
    queryKey: keys.roles(workspaceId),
    queryFn: ({ signal }) => listRoles(workspaceId, signal),
    enabled: workspaceId !== '' && maySeeGrants,
  })

  return (
    <>
      <People
        members={members.data?.data ?? []}
        loading={members.isPending}
        error={members.error}
        hasMore={members.data?.page.has_more ?? false}
        grants={grants.data?.data ?? []}
        roles={roles.data?.data ?? []}
        showGrants={maySeeGrants}
        mayManage={mayManage}
      />
      <Invitations mayManage={mayManage} roles={roles.data?.data ?? []} />
    </>
  )
}

function People({
  members,
  loading,
  error,
  hasMore,
  grants,
  roles,
  showGrants,
  mayManage,
}: {
  members: readonly Member[]
  loading: boolean
  error: unknown
  hasMore: boolean
  grants: readonly Assignment[]
  roles: readonly Role[]
  showGrants: boolean
  mayManage: boolean
}): ReactElement {
  const workspaceId = useWorkspaceId()
  const { actor } = useSession()

  const remove = useWrite({
    run: (userId: string) => removeMember(workspaceId, userId),
    announce: () => 'Removed from the workspace.',
    invalidates: [keys.members(workspaceId), keys.assignments(workspaceId)],
  })

  const roleName = new Map(roles.map((role) => [role.id, role.name]))
  const held = new Map<string, string[]>()
  for (const grant of grants) {
    if (grant.principal_type !== 'USER') continue
    const names = held.get(grant.principal_id) ?? []
    names.push(roleName.get(grant.role_id) ?? 'a role')
    held.set(grant.principal_id, names)
  }

  return (
    <PageHead
      title="Members"
      description="Being a member makes you visible here. What you may do comes from a role, which is a separate thing."
    >
      <WriteError error={remove.error} />
      {loading ? <Loading rows={4} label="Loading members" /> : null}
      {error ? <ErrorNotice error={error} /> : null}
      <ul className="settings__rows">
        {members.map((member) => (
          <li className="settings__row" key={member.user_id}>
            <span className="settings__row-main">
              {member.display_name}
              {member.member_type === 'GUEST' ? <Badge tone="neutral">guest</Badge> : null}
              {member.user_id === actor?.actor_id ? <Badge tone="accent">you</Badge> : null}
            </span>
            <span className="settings__row-meta">
              {member.email ?? 'address removed'}
              {showGrants ? ` · ${describeRoles(held.get(member.user_id))}` : ''}
            </span>
            {mayManage && member.user_id !== actor?.actor_id ? (
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
      {hasMore ? (
        <p className="field__hint">Showing the first 100. Narrowing this list is not built yet.</p>
      ) : null}
    </PageHead>
  )
}

/**
 * What a member's grants amount to, in a sentence.
 *
 * "No roles" is said out loud rather than left blank: someone with no grant can
 * sign in and see nothing, and a blank cell reads as "fine" rather than as the
 * thing to fix.
 */
function describeRoles(names: readonly string[] | undefined): string {
  if (names === undefined || names.length === 0)
    return 'no roles — they can sign in and see nothing'
  return names.join(', ')
}

function Invitations({
  mayManage,
  roles,
}: {
  mayManage: boolean
  roles: readonly Role[]
}): ReactElement {
  const workspaceId = useWorkspaceId()
  const [email, setEmail] = useState('')
  const [roleId, setRoleId] = useState('')

  const invitations = useQuery({
    queryKey: keys.invitations(workspaceId),
    queryFn: ({ signal }) => listInvitations(workspaceId, signal),
    enabled: workspaceId !== '' && mayManage,
  })

  const invite = useWrite({
    run: () =>
      createInvitation(workspaceId, {
        email: email.trim(),
        role_id: roleId === '' ? undefined : roleId,
      }),
    announce: (created) => `Invitation sent to ${created.email}.`,
    invalidates: [keys.invitations(workspaceId)],
    onDone: () => setEmail(''),
  })

  const revoke = useWrite({
    run: (id: string) => revokeInvitation(workspaceId, id),
    announce: () => 'Invitation revoked.',
    invalidates: [keys.invitations(workspaceId)],
  })

  if (!mayManage) {
    return (
      <Section
        title="Invitations"
        description="Who has been asked to join, and has not arrived yet."
      >
        <NeedsPermission permission="workspace.manage" />
      </Section>
    )
  }

  const pending = invitations.data?.data ?? []

  return (
    <Section
      title="Invitations"
      description="Who has been asked to join and has not arrived yet. An invitation can carry the role they will hold."
    >
      <Form onSubmit={() => invite.submit(undefined)}>
        <Field label="Email address" id="invite-email">
          <Input
            full
            id="invite-email"
            type="email"
            value={email}
            onChange={(event) => setEmail(event.target.value)}
          />
        </Field>
        <Field
          label="Role on arrival"
          id="invite-role"
          hint="Optional. Checked against your own authority now, so an invitation cannot grant what you could not."
        >
          <Select
            full
            id="invite-role"
            value={roleId}
            onChange={(event) => setRoleId(event.target.value)}
          >
            <option value="">No role — they arrive able to see nothing</option>
            {roles.map((role) => (
              <option key={role.id} value={role.id}>
                {role.name}
              </option>
            ))}
          </Select>
        </Field>
        <WriteError error={invite.error} />
        <Button variant="primary" type="submit" disabled={invite.pending || email.trim() === ''}>
          {invite.pending ? 'Sending…' : 'Send invitation'}
        </Button>
      </Form>

      <WriteError error={revoke.error} />
      {invitations.isPending ? <Loading label="Loading invitations" /> : null}
      {invitations.error ? <ErrorNotice error={invitations.error} /> : null}
      {!invitations.isPending && pending.length === 0 ? (
        <p className="empty">Nobody is waiting on an invitation.</p>
      ) : null}
      <ul className="settings__rows">
        {pending.map((row) => (
          <InvitationRow
            key={row.id}
            invitation={row}
            roleName={roles.find((role) => role.id === row.role_id)?.name}
            onRevoke={() => revoke.submit(row.id)}
            busy={revoke.pending}
          />
        ))}
      </ul>
    </Section>
  )
}

function InvitationRow({
  invitation,
  roleName,
  onRevoke,
  busy,
}: {
  invitation: Invitation
  roleName: string | undefined
  onRevoke: () => void
  busy: boolean
}): ReactElement {
  const expiry = new Date(invitation.expires_at)
  const expired = !Number.isNaN(expiry.getTime()) && expiry.getTime() < Date.now()
  return (
    <li className="settings__row">
      <span className="settings__row-main">{invitation.email}</span>
      <span className="settings__row-meta">
        {roleName === undefined ? 'no role' : `arrives as ${roleName}`} ·{' '}
        {expired ? 'expired' : 'expires'}{' '}
        <time dateTime={invitation.expires_at}>{expiry.toLocaleString()}</time>
      </span>
      <Button variant="subtle" onClick={onRevoke} disabled={busy}>
        Revoke
      </Button>
    </li>
  )
}
