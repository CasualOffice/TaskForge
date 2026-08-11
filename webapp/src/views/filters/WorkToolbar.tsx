/**
 * The work toolbar: what the view is, how it is narrowed, how it is ordered.
 *
 * # The failure this module prevents
 *
 * A tracker you cannot narrow. The server has filtered correctly since C-012 —
 * status, state, type, priority, assignee, reporter, tags, dates and full text,
 * all indexed, all refusing an unknown field — and for a while the client
 * offered a project dropdown and a search box. A list of every task in a
 * workspace is not a work tracker; it is a table you scroll.
 *
 * # Ordering is specified, not chosen
 *
 * `design/LAYOUT-AND-INTERACTION-GUIDELINES.md` §5 fixes it:
 *
 *     [View identity]   [Filter] [Group] [Sort]             [Create]
 *
 * and caps a toolbar at two rows. So the controls sit in that order, `Create` is
 * pushed right by a spacer, and nothing decorative is added between them.
 *
 * **`Group` is not here yet, and is not faked.** Grouping a list means one query
 * per group — the way the board already runs one query per column — because
 * grouping the current *page* would group 100 rows out of 4,000 and label the
 * result as if it were the whole set. That is the next piece of work, not a
 * control that half-works.
 *
 * # The project control is in the identity slot on purpose, and temporarily
 *
 * §3 makes navigation `My Work · Projects · Search · Activity`, with board and
 * list as *views of a project* — so choosing a project becomes navigation rather
 * than a filter. Until that restructure lands, the project control occupies the
 * view-identity slot rather than sitting among the filters, so the move is a
 * change of component and not a change of layout.
 *
 * # Everything here writes to the URL
 *
 * See `router.tsx` §AppSearch. The consequence worth stating: this component
 * holds no filter state at all. It reads the address and writes the address, so
 * the back button undoes a filter, a reload keeps it, and a link carries it.
 */
import { useEffect, useState, type ReactElement, type ReactNode } from 'react'
import { useQuery } from '@tanstack/react-query'
import { Input, Select } from '@schnsrw/design-system'

import { keys } from '../../api/keys'
import { listProjects } from '../../api/projects'
import { PRIORITIES, SORT_KEYS, TASK_TYPES, type Sort, type SortKey } from '../../api/tasks'
import type { Workflow } from '../../api/workflow'
import { directory, listMembers } from '../../api/workspaces'
import { useAppSearch, useUpdateSearch } from '../../shell/navigation'
import { useWorkspaceId } from '../../shell/session'
import { priorityLabel, typeLabel } from '../../tasks/present'
import { DUE_PRESETS, NO_FILTERS } from '../../tasks/query'
import type { BuiltinView } from '../../tasks/builtinViews'
import { useProjectWorkflow } from '../../tasks/useWorkflow'
import { ActiveFilters } from './ActiveFilters'
import { describeFilters } from './describe'
import { CONTROL, narrowing } from '../../shell/controls'
import { FilterMenu, FilterSelect, type FilterOption } from './FilterMenu'
import { MoreFilters } from './MoreFilters'
import { ViewsMenu } from './ViewsMenu'
import { GROUP_KEYS, GROUP_LABELS, groupUnavailable, type GroupKey } from '../list/grouping'

/** Long enough that a typist does not fire a request per letter, short enough to feel live. */
const DEBOUNCE_MS = 250

/** Sentinels for the assignee control, which has three meanings a uuid cannot carry. */
const ANYONE = '__anyone'
const UNASSIGNED = '__unassigned'

const SORT_LABELS: Readonly<Record<SortKey, string>> = {
  updated_at: 'Last updated',
  created_at: 'Created',
  due_at: 'Due date',
  priority: 'Priority',
  position: 'Board order',
  key: 'Identifier',
}

export function WorkToolbar({
  sort,
  onSort,
  group,
  onGroup,
  workflow: groupWorkflow,
  onColumns,
  children,
}: {
  /** Absent on the board, which is ordered by board rank and by nothing else. */
  sort?: Sort
  onSort?: (next: Sort) => void
  /** Absent on the board, whose columns already are the grouping. */
  group?: GroupKey | undefined
  onGroup?: (next: GroupKey | undefined) => void
  /** Needed to name the status groups, and to say why they are unavailable. */
  workflow?: Workflow | undefined
  /**
   * Fields the surface filters on its own column headings.
   *
   * The list puts its filters on the columns they narrow, which is where every
   * application of this kind puts them — a toolbar dropdown makes a reader map
   * "Status" in one place onto a column somewhere else. The board and the
   * report have no column headings, so they keep theirs here, and the toolbar
   * is told rather than guessing which surface it is on.
   */
  onColumns?: readonly ('status' | 'priority' | 'type')[]
  /** The create control, which the ordering puts last. */
  children?: ReactNode
}): ReactElement {
  const workspaceId = useWorkspaceId()
  const search = useAppSearch()
  const owns = (field: 'status' | 'priority' | 'type'): boolean =>
    onColumns?.includes(field) ?? false
  const update = useUpdateSearch()
  const { workflow } = useProjectWorkflow(search.project)

  const projects = useQuery({
    queryKey: keys.projects(workspaceId),
    queryFn: ({ signal }) => listProjects(workspaceId, signal),
    enabled: workspaceId !== '',
    staleTime: 60_000,
  })

  const members = useQuery({
    queryKey: keys.members(workspaceId),
    queryFn: ({ signal }) => listMembers(workspaceId, signal),
    enabled: workspaceId !== '',
    staleTime: 5 * 60_000,
  })

  const [term, setTerm] = useState(search.q ?? '')

  // The URL is the authority: a back button or a pasted link changes `search.q`
  // without going through this input, and a box that ignored that would show one
  // term while the list showed another.
  useEffect(() => setTerm(search.q ?? ''), [search.q])

  useEffect(() => {
    const current = search.q ?? ''
    if (term === current) return
    const timer = setTimeout(() => update({ q: term === '' ? undefined : term }), DEBOUNCE_MS)
    return () => clearTimeout(timer)
  }, [term, search.q, update])

  const statusOptions: FilterOption[] = (workflow?.statuses ?? []).map((status) => ({
    value: status.id,
    label: status.name,
  }))

  // Reversed so the severities people filter for are first. `NONE` is offered
  // because "what has nobody triaged?" is a real question.
  const priorityOptions: FilterOption[] = [...PRIORITIES]
    .reverse()
    .map((priority) => ({ value: priority, label: priorityLabel(priority) }))

  const typeOptions: FilterOption[] = TASK_TYPES.map((type) => ({
    value: type,
    label: typeLabel(type),
  }))

  const assignee =
    search.assignee === undefined ? ANYONE : search.assignee === '' ? UNASSIGNED : search.assignee

  const nameOf = directory(members.data?.data ?? [])
  const statusName = (id: string): string =>
    workflow?.statuses.find((status) => status.id === id)?.name ?? id

  const applied = describeFilters(search, { status: statusName, person: nameOf })

  /** Replace every filter at once — a view is a whole filter, not an addition. */
  function applyView(view: BuiltinView): void {
    update({ ...NO_FILTERS, ...view.search })
  }

  return (
    <div className="toolbar">
      {/* ── View identity ──────────────────────────────────────────────── */}
      <ViewsMenu search={search} onApply={applyView} />
      <label className="visually-hidden" htmlFor="scope-project">
        Project
      </label>
      <Select
        width="auto"
        containerStyle={{ maxWidth: 260 }}
        style={{ height: CONTROL, fontWeight: 600 }}
        id="scope-project"
        value={search.project ?? ''}
        onChange={(event) =>
          // Statuses belong to a project's workflow, so a status id from the old
          // project would filter the new one to nothing. Cleared with the switch
          // rather than left to produce an empty board nobody can explain.
          update({
            project: event.target.value === '' ? undefined : event.target.value,
            status: undefined,
          })
        }
      >
        <option value="">All projects</option>
        {(projects.data?.data ?? []).map((project) => (
          <option key={project.id} value={project.id}>
            {project.key} — {project.name}
          </option>
        ))}
      </Select>

      {/* ── Filter ─────────────────────────────────────────────────────── */}
      <div className="toolbar__group">
        <label className="visually-hidden" htmlFor="scope-search">
          Search tasks
        </label>
        <Input
          style={{ width: 'min(240px, 34vw)' }}
          id="scope-search"
          type="search"
          placeholder="Search…"
          value={term}
          onChange={(event) => setTerm(event.target.value)}
        />

        {statusOptions.length === 0 || owns('status') ? null : (
          <FilterMenu
            label="Status"
            options={statusOptions}
            selected={search.status}
            onChange={(next) => update({ status: next })}
          />
        )}

        {owns('priority') ? null : (
          <FilterMenu
            label="Priority"
            options={priorityOptions}
            selected={search.priority}
            onChange={(next) => update({ priority: next })}
          />
        )}

        {owns('type') ? null : (
          <FilterMenu
            label="Type"
            options={typeOptions}
            selected={search.type}
            onChange={(next) => update({ type: next })}
          />
        )}

        <label className="visually-hidden" htmlFor="filter-assignee">
          Assignee
        </label>
        <Select
          full
          id="filter-assignee"
          style={{ height: CONTROL, ...narrowing(search.assignee !== undefined) }}
          value={assignee}
          onChange={(event) => {
            const chosen = event.target.value
            // Three meanings, and only one of them is a user id. `assignee=` is
            // the grammar's `is_empty`, so "unassigned" is a present-and-empty
            // value — not an absent one, which means "anyone".
            update({
              assignee: chosen === ANYONE ? undefined : chosen === UNASSIGNED ? '' : chosen,
            })
          }}
        >
          <option value={ANYONE}>Anyone</option>
          <option value="@me">Me</option>
          <option value={UNASSIGNED}>Unassigned</option>
          {(members.data?.data ?? []).map((member) => (
            <option key={member.user_id} value={member.user_id}>
              {member.display_name}
            </option>
          ))}
        </Select>

        <FilterSelect
          label="Due"
          options={DUE_PRESETS}
          value={search.due}
          onChange={(next) => update({ due: next })}
        />

        <MoreFilters search={search} onChange={update} />
      </div>

      {/* ── Group ──────────────────────────────────────────────────────── */}
      {onGroup === undefined ? null : (
        <>
          <label className="visually-hidden" htmlFor="toolbar-group">
            Group by
          </label>
          <Select
            width="auto"
            containerStyle={{ maxWidth: 180 }}
            style={{ height: CONTROL, ...narrowing(group !== undefined) }}
            id="toolbar-group"
            value={group ?? ''}
            onChange={(event) =>
              onGroup(event.target.value === '' ? undefined : (event.target.value as GroupKey))
            }
          >
            <option value="">No grouping</option>
            {GROUP_KEYS.map((key) => {
              // A key whose values cannot be enumerated yet is offered disabled
              // with the reason, rather than silently absent — a missing option
              // reads as a capability the product lacks.
              const why = groupUnavailable(key, groupWorkflow)
              return (
                <option key={key} value={key} disabled={why !== undefined}>
                  {`Group by ${GROUP_LABELS[key].toLowerCase()}`}
                  {why === undefined ? '' : ' — needs a project'}
                </option>
              )
            })}
          </Select>
        </>
      )}

      {/* ── Sort ───────────────────────────────────────────────────────── */}
      {sort === undefined || onSort === undefined ? null : (
        <>
          <label className="visually-hidden" htmlFor="toolbar-sort">
            Sort by
          </label>
          <Select
            width="auto"
            containerStyle={{ maxWidth: 200 }}
            style={{ height: CONTROL }}
            id="toolbar-sort"
            value={`${sort.descending ? '-' : ''}${sort.key}`}
            onChange={(event) => {
              const raw = event.target.value
              const descending = raw.startsWith('-')
              onSort({ key: (descending ? raw.slice(1) : raw) as SortKey, descending })
            }}
          >
            {SORT_KEYS.flatMap((key) => [
              <option key={`-${key}`} value={`-${key}`}>
                {SORT_LABELS[key]} ↓
              </option>,
              <option key={key} value={key}>
                {SORT_LABELS[key]} ↑
              </option>,
            ])}
          </Select>
        </>
      )}

      <span className="shell__spacer" />
      {children}

      {/* Row two, and only when there is something to say. design/LAYOUT §5
          caps a toolbar at two rows; the second is the one that answers "why is
          this list empty?". */}
      <ActiveFilters
        filters={applied}
        onRemove={(key) => update({ [key]: undefined })}
        // `project` survives: dropping it would empty the board and unset the
        // workflow, which is not what anyone means by "clear".
        onClear={() => update(NO_FILTERS)}
      />
    </div>
  )
}
