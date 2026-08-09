/**
 * `/settings/roles` — what each role carries, and who holds it.
 *
 * # Two permissions, two halves of this screen
 *
 * `docs/04` (D-049) splits authoring a role (`role.manage`) from granting one
 * (`role.assign`), because merged, anyone who could assign could also mint a
 * role more powerful than their own and give it to themselves. The screen keeps
 * the split visible: the editor and the grant list are separate sections with
 * separate refusals, so someone who may only assign is not shown an editor whose
 * every save is a `403`.
 *
 * # Why the ceilings are not pre-checked here
 *
 * `docs/04`'s five controls are resolved server-side against the actor's own
 * grants, and each refusal has its own code — `TF-AZN-0003` (you do not hold
 * it), `TF-AZN-0004` (above your scope), `TF-AZN-0006` (you cannot give it to
 * yourself). Reimplementing them in TypeScript would be a second copy of the
 * rule that drifts, and would hide the *reason* behind a greyed-out control. So
 * the control is offered and the refusal is rendered, naming the rule.
 *
 * # Why a permission set is replaced, never merged
 *
 * Control 1 re-checks the ceiling on edit against the set it was given. A merge
 * would let a permission survive that the check never saw, which is exactly the
 * smuggling the control forbids — so the editor sends the whole set every time.
 */
import { useState, type ReactElement } from 'react'
import { useQuery } from '@tanstack/react-query'

import { listMembers, listTeams } from '../api/admin'
import { keys } from '../api/keys'
import {
  assignRole,
  createRole,
  listAssignments,
  listRoles,
  PERMISSION_GROUPS,
  PERMISSION_HELP,
  revokeAssignment,
  updateRole,
  type Assignment,
  type PrincipalType,
  type Role,
} from '../api/roles'
import { useAuthority } from '../shell/permissions'
import { useWorkspaceId } from '../shell/session'
import { ErrorNotice } from '../shell/notice'
import { Field, Form, Loading, NeedsPermission, Section, useWrite, WriteError } from './parts'

export function RolesSettings(): ReactElement {
  const workspaceId = useWorkspaceId()
  const authority = useAuthority()
  const mayAuthor = authority.can('role.manage')
  const mayAssign = authority.can('role.assign')

  const roles = useQuery({
    queryKey: keys.roles(workspaceId),
    queryFn: ({ signal }) => listRoles(workspaceId, signal),
    enabled: workspaceId !== '' && (mayAuthor || mayAssign),
  })

  if (authority.loading) return <Loading rows={5} label="Loading roles" />
  if (!mayAuthor && !mayAssign) return <NeedsPermission permission="role.assign" />
  if (roles.error) return <ErrorNotice error={roles.error} />

  const list = roles.data?.data ?? []

  return (
    <>
      <Section
        title="Roles"
        description="A role is a named set of permissions. It does nothing until it is granted to someone, at some scope."
      >
        {roles.isPending ? <Loading rows={4} label="Loading roles" /> : null}
        <ul className="settings__rows">
          {list.map((role) => (
            <RoleRow key={role.id} role={role} mayAuthor={mayAuthor} />
          ))}
        </ul>
        {mayAuthor ? <NewRole /> : null}
      </Section>
      <Grants roles={list} mayAssign={mayAssign} />
    </>
  )
}

function RoleRow({ role, mayAuthor }: { role: Role; mayAuthor: boolean }): ReactElement {
  const [editing, setEditing] = useState(false)
  return (
    <li className="settings__row settings__row--stacked">
      <div className="settings__row-head">
        <span className="settings__row-main">
          {role.name}
          {role.is_template ? <span className="badge"> template</span> : null}
        </span>
        <span className="settings__row-meta">
          {role.permissions.length === 0
            ? 'carries nothing'
            : `${role.permissions.length} ${role.permissions.length === 1 ? 'permission' : 'permissions'}`}
        </span>
        {mayAuthor ? (
          <button
            type="button"
            className="button button--quiet"
            aria-expanded={editing}
            onClick={() => setEditing(!editing)}
          >
            {editing ? 'Cancel' : 'Edit'}
          </button>
        ) : null}
      </div>
      {editing ? (
        <RoleEditor role={role} onDone={() => setEditing(false)} />
      ) : (
        <p className="settings__row-meta">
          {role.permissions.length === 0
            ? 'Anyone holding this role can do nothing with it.'
            : role.permissions.join(', ')}
        </p>
      )}
    </li>
  )
}

function RoleEditor({ role, onDone }: { role: Role; onDone: () => void }): ReactElement {
  const workspaceId = useWorkspaceId()
  const [name, setName] = useState(role.name)
  const [chosen, setChosen] = useState<ReadonlySet<string>>(new Set(role.permissions))

  const save = useWrite({
    run: () =>
      updateRole(workspaceId, role.id, role.version, {
        name: name.trim(),
        // The whole set, always. See the module comment: a partial send would
        // be a merge, and control 1 checks what it was given.
        permissions: [...chosen],
      }),
    announce: (updated) => `Saved ${updated.name}.`,
    invalidates: [keys.roles(workspaceId), keys.workspace(workspaceId)],
    onDone,
  })

  return (
    <Form onSubmit={() => save.submit(undefined)}>
      <Field label="Name" id={`role-name-${role.id}`}>
        <input
          id={`role-name-${role.id}`}
          className="input"
          value={name}
          maxLength={200}
          onChange={(event) => setName(event.target.value)}
        />
      </Field>
      <PermissionPicker chosen={chosen} onChange={setChosen} idPrefix={role.id} />
      <WriteError error={save.error} />
      <button
        type="submit"
        className="button button--primary"
        disabled={save.pending || name.trim() === ''}
      >
        {save.pending ? 'Saving…' : 'Save role'}
      </button>
    </Form>
  )
}

function NewRole(): ReactElement {
  const workspaceId = useWorkspaceId()
  const [name, setName] = useState('')
  const [chosen, setChosen] = useState<ReadonlySet<string>>(new Set())
  const [open, setOpen] = useState(false)

  const create = useWrite({
    run: () => createRole(workspaceId, { name: name.trim(), permissions: [...chosen] }),
    announce: (role) => `Created the role ${role.name}.`,
    invalidates: [keys.roles(workspaceId)],
    onDone: () => {
      setName('')
      setChosen(new Set())
      setOpen(false)
    },
  })

  if (!open) {
    return (
      <button type="button" className="button" onClick={() => setOpen(true)}>
        New role
      </button>
    )
  }

  return (
    <Form onSubmit={() => create.submit(undefined)}>
      <Field
        label="Name"
        id="new-role-name"
        hint="You can only give a new role permissions you hold yourself."
      >
        <input
          id="new-role-name"
          className="input"
          value={name}
          maxLength={200}
          onChange={(event) => setName(event.target.value)}
        />
      </Field>
      <PermissionPicker chosen={chosen} onChange={setChosen} idPrefix="new" />
      <WriteError error={create.error} />
      <button
        type="submit"
        className="button button--primary"
        disabled={create.pending || name.trim() === ''}
      >
        {create.pending ? 'Creating…' : 'Create role'}
      </button>
      <button type="button" className="button button--quiet" onClick={() => setOpen(false)}>
        Cancel
      </button>
    </Form>
  )
}

/**
 * Every permission, grouped, each with what it actually allows.
 *
 * Checkboxes rather than a multi-select: a permission set is read far more often
 * than it is edited, and a list of ticked boxes with sentences beside them can be
 * *read*. A multi-select of thirty opaque keys cannot.
 */
function PermissionPicker({
  chosen,
  onChange,
  idPrefix,
}: {
  chosen: ReadonlySet<string>
  onChange: (next: ReadonlySet<string>) => void
  idPrefix: string
}): ReactElement {
  const toggle = (key: string): void => {
    const next = new Set(chosen)
    if (next.has(key)) next.delete(key)
    else next.add(key)
    onChange(next)
  }

  return (
    <div className="settings__permissions">
      {PERMISSION_GROUPS.map((group) => (
        <fieldset className="settings__permission-group" key={group.title}>
          <legend className="field__label">{group.title}</legend>
          {group.keys.map((key) => (
            <label className="settings__permission" key={key} htmlFor={`${idPrefix}-${key}`}>
              <input
                id={`${idPrefix}-${key}`}
                type="checkbox"
                checked={chosen.has(key)}
                onChange={() => toggle(key)}
              />
              <span className="settings__permission-key">{key}</span>
              <span className="settings__permission-help">{PERMISSION_HELP[key] ?? ''}</span>
            </label>
          ))}
        </fieldset>
      ))}
    </div>
  )
}

/**
 * Who holds what.
 *
 * The scope is workspace-only here. `docs/04` allows team, project and
 * environment scopes and the API takes them; a picker for those needs a project
 * and environment chooser this screen does not have yet, and offering a scope
 * whose id the user must paste would be worse than not offering it. Said out
 * loud below rather than left to be discovered.
 */
function Grants({ roles, mayAssign }: { roles: readonly Role[]; mayAssign: boolean }): ReactElement {
  const workspaceId = useWorkspaceId()
  const [principalType, setPrincipalType] = useState<PrincipalType>('USER')
  const [principalId, setPrincipalId] = useState('')
  const [roleId, setRoleId] = useState('')

  const grants = useQuery({
    queryKey: keys.assignmentsFor(workspaceId, { scope: 'all' }),
    queryFn: ({ signal }) => listAssignments(workspaceId, {}, signal),
    enabled: workspaceId !== '',
  })
  const members = useQuery({
    queryKey: keys.members(workspaceId),
    queryFn: ({ signal }) => listMembers(workspaceId, undefined, signal),
    enabled: workspaceId !== '',
  })
  const teams = useQuery({
    queryKey: keys.teams(workspaceId),
    queryFn: ({ signal }) => listTeams(workspaceId, signal),
    enabled: workspaceId !== '',
  })

  const grant = useWrite({
    run: () =>
      assignRole(workspaceId, {
        principal_type: principalType,
        principal_id: principalId,
        role_id: roleId,
        scope_type: 'WORKSPACE',
      }),
    announce: () => 'Granted.',
    invalidates: [keys.assignments(workspaceId), keys.workspace(workspaceId)],
    onDone: () => setPrincipalId(''),
  })

  const revoke = useWrite({
    run: (id: string) => revokeAssignment(workspaceId, id),
    announce: () => 'Grant revoked.',
    invalidates: [keys.assignments(workspaceId), keys.workspace(workspaceId)],
  })

  const nameOf = (assignment: Assignment): string => {
    if (assignment.principal_type === 'TEAM') {
      const team = (teams.data?.data ?? []).find((t) => t.id === assignment.principal_id)
      return team === undefined ? assignment.principal_id : `${team.name} (team)`
    }
    const member = (members.data?.data ?? []).find((m) => m.user_id === assignment.principal_id)
    return member?.display_name ?? assignment.principal_id
  }
  const roleOf = (assignment: Assignment): string =>
    roles.find((role) => role.id === assignment.role_id)?.name ?? assignment.role_id

  const rows = grants.data?.data ?? []
  const principals =
    principalType === 'TEAM'
      ? (teams.data?.data ?? []).map((team) => ({ id: team.id, label: team.name }))
      : (members.data?.data ?? []).map((member) => ({
          id: member.user_id,
          label: `${member.display_name} · ${member.email ?? 'address removed'}`,
        }))

  return (
    <Section
      title="Who holds what"
      description="Every grant in this workspace. A grant is a role given to a person or a team, at a scope."
    >
      {mayAssign ? (
        <Form onSubmit={() => grant.submit(undefined)}>
          <Field label="Give it to" id="grant-principal-type">
            <select
              id="grant-principal-type"
              className="select"
              value={principalType}
              onChange={(event) => {
                setPrincipalType(event.target.value as PrincipalType)
                setPrincipalId('')
              }}
            >
              <option value="USER">A person</option>
              <option value="TEAM">A team — everyone in it inherits</option>
            </select>
          </Field>
          <Field label={principalType === 'TEAM' ? 'Team' : 'Person'} id="grant-principal">
            <select
              id="grant-principal"
              className="select"
              value={principalId}
              onChange={(event) => setPrincipalId(event.target.value)}
            >
              <option value="">Choose…</option>
              {principals.map((option) => (
                <option key={option.id} value={option.id}>
                  {option.label}
                </option>
              ))}
            </select>
          </Field>
          <Field
            label="Role"
            id="grant-role"
            hint="Granted across the whole workspace. Narrower scopes exist in the API and have no picker here yet."
          >
            <select
              id="grant-role"
              className="select"
              value={roleId}
              onChange={(event) => setRoleId(event.target.value)}
            >
              <option value="">Choose…</option>
              {roles.map((role) => (
                <option key={role.id} value={role.id}>
                  {role.name}
                </option>
              ))}
            </select>
          </Field>
          <WriteError error={grant.error} />
          <button
            type="submit"
            className="button button--primary"
            disabled={grant.pending || principalId === '' || roleId === ''}
          >
            {grant.pending ? 'Granting…' : 'Grant role'}
          </button>
        </Form>
      ) : (
        <NeedsPermission permission="role.assign" />
      )}

      <WriteError error={revoke.error} />
      {grants.isPending ? <Loading rows={4} label="Loading grants" /> : null}
      {grants.error ? <ErrorNotice error={grants.error} /> : null}
      {!grants.isPending && rows.length === 0 ? (
        <p className="empty">Nobody holds anything. That cannot be right — someone owns this workspace.</p>
      ) : null}
      <ul className="settings__rows">
        {rows.map((row) => (
          <li className="settings__row" key={row.id}>
            <span className="settings__row-main">
              {nameOf(row)} — {roleOf(row)}
            </span>
            <span className="settings__row-meta">
              at {row.scope_type.toLowerCase()} scope · granted{' '}
              <time dateTime={row.granted_at}>{new Date(row.granted_at).toLocaleDateString()}</time>
            </span>
            {mayAssign ? (
              <button
                type="button"
                className="button button--quiet"
                onClick={() => revoke.submit(row.id)}
                disabled={revoke.pending}
              >
                Revoke
              </button>
            ) : null}
          </li>
        ))}
      </ul>
      {grants.data?.page.has_more === true ? (
        <p className="field__hint">Showing the first 100 grants.</p>
      ) : null}
    </Section>
  )
}
