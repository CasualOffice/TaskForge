/**
 * Grouping a list, and why only closed sets are offered.
 *
 * # The failure this module prevents
 *
 * Grouping the page instead of the result. The obvious implementation takes the
 * hundred rows already fetched and buckets them in the browser — and then labels
 * a bucket "Urgent · 4" when the workspace has ninety urgent tasks. A count that
 * describes the page while claiming to describe the query is worse than no
 * grouping at all, because it is only wrong for the customers with enough data
 * to need it.
 *
 * So a group is a *query*, exactly as a board column is: one keyset-paged
 * request per group, filtered on that group's value, with its own cursor.
 *
 * # Only closed sets are groupable
 *
 * A group per value means the set of values has to be known before the first
 * request, and small enough that N queries is reasonable. `status`, `state`,
 * `type` and `priority` are closed enumerations — five or six each. `assignee`
 * is not: it is every member of the workspace, so grouping by it would fire one
 * query per person and still miss "unassigned". `tag` is worse and has no
 * endpoint to enumerate it at all.
 *
 * Offering those anyway and quietly capping them would be the page-grouping bug
 * wearing a different hat, so they are not offered.
 */
import type { Workflow } from '../../api/workflow'
import { PRIORITIES, TASK_STATES, TASK_TYPES } from '../../api/tasks'
import type { AppSearch } from '../../router'
import { priorityLabel, stateLabel, typeLabel } from '../../tasks/present'

/** The fields a list may be grouped by. */
export const GROUP_KEYS = ['status', 'state', 'type', 'priority'] as const
export type GroupKey = (typeof GROUP_KEYS)[number]

export const GROUP_LABELS: Readonly<Record<GroupKey, string>> = {
  status: 'Status',
  state: 'State',
  type: 'Type',
  priority: 'Priority',
}

export interface Group {
  /** Stable across renders — used as a React key and as part of the query key. */
  readonly id: string
  readonly title: string
  /** The search parameters that scope this group, merged over the view's own. */
  readonly scope: Partial<AppSearch>
}

/**
 * The groups for a key, in the order they should read.
 *
 * `status` needs the workflow, so it yields nothing without one — a list with no
 * project scoped cannot group by status, and the control says so rather than
 * rendering a single group called "undefined".
 */
export function groupsFor(key: GroupKey, workflow: Workflow | undefined): readonly Group[] {
  if (key === 'status') {
    if (workflow === undefined) return []
    return [...workflow.statuses]
      .sort((a, b) => a.position - b.position)
      .map((status) => ({ id: status.id, title: status.name, scope: { status: status.id } }))
  }
  if (key === 'state') {
    return TASK_STATES.map((state) => ({
      id: state,
      title: stateLabel(state),
      scope: { state },
    }))
  }
  if (key === 'type') {
    return TASK_TYPES.map((type) => ({ id: type, title: typeLabel(type), scope: { type } }))
  }
  // Highest first: a grouped list is read top-down, and the question behind
  // grouping by priority is "what is most urgent", not "what is least".
  return [...PRIORITIES].reverse().map((priority) => ({
    id: priority,
    title: priorityLabel(priority),
    scope: { priority },
  }))
}

/** Whether a group key can be used right now, and why not when it cannot. */
export function groupUnavailable(key: GroupKey, workflow: Workflow | undefined): string | undefined {
  if (key === 'status' && workflow === undefined) {
    return 'Statuses belong to a project’s workflow — choose a project first.'
  }
  return undefined
}
