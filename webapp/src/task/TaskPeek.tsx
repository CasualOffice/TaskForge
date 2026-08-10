/**
 * `TaskPeek` — a quick look at a task without leaving the board or the list.
 *
 * # What this is not
 *
 * It is not a smaller detail page. `design/LAYOUT-AND-INTERACTION-GUIDELINES.md`
 * §4: "Use the drawer only for a **peek** … showing identity, state, assignee
 * and description, with an obvious route to the full view. It is not a smaller
 * version of the detail page; it is a different, deliberately partial thing."
 *
 * The distinction matters because the previous drawer tried to be the whole
 * detail surface in 480 px and produced the failure the specification now names:
 * a reader had to scroll to learn the assignee, and the conversation, the
 * relations and the dates were all below the fold.
 *
 * So the peek shows **only what fits**: who, what state, when it is due, and the
 * opening of the description. Everything else is one click away on a surface
 * built to hold it. It does not scroll, by construction — if a field would not
 * fit, it does not belong here.
 *
 * # It shares its parts with the full route
 *
 * §9 of the foundation requires one component set across the two, so a field
 * cannot exist in one and be forgotten in the other. `StatusControl` and the
 * assignee row here are the same components the detail route renders.
 */
import { Button } from '@schnsrw/design-system'
import { useCallback, useRef, type ReactElement } from 'react'
import { Link } from '@tanstack/react-router'
import { readAssignees } from '../api/tasks'
import { listTeams } from '../api/admin'
import { listEnvironments } from '../api/environments'
import { useQuery } from '@tanstack/react-query'

import { keys } from '../api/keys'
import { readTask } from '../api/tasks'
import { directory, listMembers } from '../api/workspaces'
import { useFocusTrap } from '../shell/focusTrap'
import { useOpenTask } from '../shell/navigation'
import { ErrorNotice } from '../shell/notice'
import { useAuthority } from '../shell/permissions'
import { useWorkspaceId } from '../shell/session'
import { formatRelative, isOverdue } from '../tasks/present'
import { PriorityBadge, TypeBadge } from '../tasks/TaskCard'
import { StatusControl } from './StatusControl'

/** How much of a description a peek shows before deferring to the full view. */
const SNIPPET = 320

export function TaskPeek({ taskId }: { taskId: string }): ReactElement {
  const workspaceId = useWorkspaceId()
  const openTask = useOpenTask()
  const panel = useRef<HTMLDivElement>(null)

  const close = useCallback(() => openTask(undefined), [openTask])
  useFocusTrap(panel, close)

  const task = useQuery({
    queryKey: keys.task(workspaceId, taskId),
    queryFn: ({ signal }) => readTask(workspaceId, taskId, signal),
    enabled: workspaceId !== '',
  })

  const members = useQuery({
    queryKey: keys.members(workspaceId),
    queryFn: ({ signal }) => listMembers(workspaceId, signal),
    enabled: workspaceId !== '',
    staleTime: 5 * 60_000,
  })
  // The two vocabularies the standing line reads through. Cached like every
  // other configuration list, and fetched only while a peek is open.
  const teams = useQuery({
    queryKey: keys.teams(workspaceId),
    queryFn: ({ signal }) => listTeams(workspaceId, signal),
    enabled: workspaceId !== '' && taskId !== undefined,
    staleTime: 5 * 60_000,
  })
  const environments = useQuery({
    queryKey: keys.environments(workspaceId, task.data?.project_id ?? ''),
    queryFn: ({ signal }) => listEnvironments(workspaceId, task.data?.project_id ?? '', signal),
    enabled: workspaceId !== '' && task.data?.project_id !== undefined,
    staleTime: 5 * 60_000,
  })

  const nameOf = directory(members.data?.data ?? [])
  const teamName = (id: string): string =>
    (teams.data?.data ?? []).find((team) => team.id === id)?.name ?? 'a team'
  const envName = (id: string): string =>
    (environments.data?.data ?? []).find((env) => env.id === id)?.name ?? 'an environment'

  const authority = useAuthority(task.data?.project_id)
  const current = task.data
  /** Present the day `TaskView` carries the set; see `TaskMeta`. */
  // Read, not cast. This used to reach for `task.assignees`, which the payload
  // has never carried — so the expression was `undefined` on every render and
  // the panel said "Not shown yet" forever. A cast that invents a field is a
  // gap that reports itself as a maybe.
  const assigneeSet = useQuery({
    queryKey: keys.assignees(workspaceId, taskId ?? ''),
    queryFn: ({ signal }) => readAssignees(workspaceId, taskId ?? '', signal),
    enabled: workspaceId !== '' && taskId !== undefined,
  })
  const assignees = assigneeSet.data?.assignees
  const description = current?.description ?? ''
  const clipped = description.length > SNIPPET

  return (
    <div className="peek">
      {/* The scrim closes on click but is `aria-hidden`: it is a target, not a
          control, and announcing it would put a nameless button in the reading
          order. Escape is the keyboard equivalent, wired by the focus trap. */}
      <div className="peek__scrim" onClick={close} aria-hidden="true" />

      <div
        className="peek__panel"
        ref={panel}
        role="dialog"
        aria-modal="true"
        aria-labelledby="peek-title"
        tabIndex={-1}
      >
        <header className="peek__head">
          <span className="key">{current?.key ?? '…'}</span>
          <span className="shell__spacer" />
          <Button variant="subtle" onClick={close}>
            Close
          </Button>
        </header>

        {task.isPending ? <p className="empty">Loading task…</p> : null}
        {task.error != null ? (
          <div className="peek__body">
            <ErrorNotice error={task.error} />
          </div>
        ) : null}

        {current === undefined ? null : (
          <div className="peek__body">
            <h2 id="peek-title" className="peek__title">
              {current.title}
            </h2>

            {/* `docs/46` §4.1: the standing line before any field. The two
                clocks are what this product knows that a status column cannot
                say, so they get the line under the title rather than a row in a
                list of eight. */}
            <p className="standing peek__standing">
              {current.team_id === null ? (
                <span className="standing__untriaged">Untriaged</span>
              ) : (
                <>
                  In <strong>{teamName(current.team_id)}</strong>&rsquo;s court
                </>
              )}
              <span className="standing__sep" aria-hidden="true">
                ·
              </span>
              {current.environment_id === null ? (
                <span className="standing__quiet">not deployed</span>
              ) : (
                <>
                  on <strong>{envName(current.environment_id)}</strong>
                </>
              )}
            </p>

            {/* A property band, not eight label/value rows. `docs/46` §2: it is
                what lets four facts occupy one line instead of four, and the
                peek's whole budget is about five seconds. */}
            <div className="peek__signals">
              <StatusControl task={current} authority={authority} />
              <TypeBadge type={current.type} />
              {current.priority === 'NONE' ? null : <PriorityBadge priority={current.priority} />}
              <span className="peek__pill">
                {assignees === undefined
                  ? '…'
                  : assignees.length === 0
                    ? 'Nobody assigned'
                    : assignees.map(nameOf).join(', ')}
              </span>
              {current.due_at === null ? null : (
                <span className={`peek__due${isOverdue(current) ? ' peek__due--late' : ''}`}>
                  {isOverdue(current) ? 'overdue ' : 'due '}
                  <time dateTime={current.due_at}>{formatRelative(current.due_at)}</time>
                </span>
              )}
            </div>

            {description === '' ? (
              <p className="narr__none">No description.</p>
            ) : (
              <p className="peek__desc">
                {clipped ? `${description.slice(0, SNIPPET).trimEnd()}…` : description}
              </p>
            )}

            {/* The obvious route §4 asks for. A `Link`, so middle-click and
                ⌘-click open a real tab — the reason this is a route at all. */}
            <Link
              to="/tasks/$taskId"
              params={{ taskId: current.id }}
              className="linkbutton linkbutton--primary peek__open"
            >
              Open full view
            </Link>
          </div>
        )}
      </div>
    </div>
  )
}
