/**
 * The built-in dashboards (`docs/38` §Built-in dashboards).
 *
 * # Why these are data and not components
 *
 * `docs/38`: "Shipped, expressed entirely in the model above — which is the
 * proof the model is sufficient. If a built-in dashboard needed a capability the
 * model lacks, that is the signal the model is under-specified."
 *
 * So every tile below is a `filter` + `measure` + `group_by` + `bucket` and
 * nothing else — the same four parts a user-composed dashboard will have, sent
 * to the same endpoint, with no private path a hand-built tile could take. When
 * saved reports arrive, these become rows rather than a rewrite.
 *
 * Writing them out did exactly what the doc predicted it would: four of the
 * tiles `docs/38` lists cannot be expressed yet, and rather than approximate
 * them they are recorded as gaps in the measure set. See §Not built below.
 *
 * # Why the filters are strings
 *
 * They are the URL form from `docs/27` — `state=BACKLOG,PLANNED,ACTIVE`,
 * `due_at=<@today`, `assignee=` for is-empty. The same grammar the address bar
 * carries, so "open" on a dashboard and "open" in the list cannot drift apart,
 * and `@me` / `@today` are resolved by the server against the reader and their
 * timezone rather than baked in here against the browser's.
 */
import type { TaskFilter } from '../../api/tasks'
import type { Bucket, Dimension, MeasureKey } from '../../api/reports'

/** The visualizations `docs/38` closes the set to, minus the one nothing uses. */
export type Viz = 'number' | 'bar' | 'line' | 'stacked_bar' | 'table'

export interface Tile {
  readonly id: string
  readonly title: string
  /** What the number means, in a sentence. Shown, not hidden in a tooltip. */
  readonly help: string
  readonly viz: Viz
  readonly filter?: TaskFilter
  readonly groupBy: Dimension
  readonly measure?: MeasureKey
  readonly bucket?: Bucket
  readonly limit?: number
  /** Columns of the 4-wide grid. A `number` tile is 1; a series wants 2. */
  readonly span: 1 | 2 | 4
}

export interface Dashboard {
  readonly id: string
  readonly name: string
  readonly description: string
  readonly tiles: readonly Tile[]
}

/**
 * Open work, in the sense every tile here means it.
 *
 * Defined once because "open" appears in eleven tiles and a dashboard whose
 * tiles disagreed about whether cancelled work is open would be worse than one
 * with no numbers at all. `CANCELED` is excluded for the reason `docs/38`
 * gives: collapsing cancelled into completed is the most common metric bug in
 * trackers, and it is why the state exists separately.
 */
const OPEN = 'BACKLOG,PLANNED,ACTIVE'

/**
 * Throughput needs an interval but ignores the field.
 *
 * `compile_throughput` buckets on `task_state_interval.entered_at` — when the
 * work actually reached `COMPLETED` — because there is no `completed_at`
 * column to truncate. The request shape still requires a field, so this names
 * the one that is least misleading if anyone reads the payload, and says here
 * that it is not what the series is bucketed by.
 */
const WEEKLY: Bucket = { field: 'created_at', interval: 'week' }

const MY_WORK: Dashboard = {
  id: 'my-work',
  name: 'My work',
  description: 'What is on you right now, and what you have been finishing.',
  tiles: [
    {
      id: 'mine-overdue',
      title: 'Overdue',
      help: 'Open tasks assigned to you whose due date has passed, in your timezone.',
      viz: 'number',
      filter: { assignee: '@me', state: OPEN, due_at: '<@today' },
      groupBy: 'state',
      span: 1,
    },
    {
      id: 'mine-open',
      title: 'Assigned to you',
      help: 'Every open task assigned to you, whatever its state.',
      viz: 'number',
      filter: { assignee: '@me', state: OPEN },
      groupBy: 'state',
      span: 1,
    },
    {
      id: 'mine-by-state',
      title: 'Where your work sits',
      help: 'Your open tasks divided by state. The parts sum to the total above.',
      viz: 'stacked_bar',
      filter: { assignee: '@me', state: OPEN },
      groupBy: 'state',
      span: 2,
    },
    {
      id: 'mine-by-priority',
      title: 'By priority',
      help: 'Your open tasks, most urgent first.',
      viz: 'bar',
      filter: { assignee: '@me', state: OPEN },
      groupBy: 'priority',
      span: 2,
    },
    {
      id: 'mine-throughput',
      title: 'Completed per week',
      help: 'Tasks assigned to you that reached a completed state, by the week they reached it. Cancelled work is not counted.',
      viz: 'line',
      filter: { assignee: '@me' },
      groupBy: 'state',
      measure: 'throughput',
      bucket: WEEKLY,
      span: 2,
    },
  ],
}

const PROJECT_HEALTH: Dashboard = {
  id: 'project-health',
  name: 'Project health',
  description: 'How fast work is moving, and where it is piling up.',
  tiles: [
    {
      id: 'health-open',
      title: 'Open',
      help: 'Every open task in the projects you can see.',
      viz: 'number',
      filter: { state: OPEN },
      groupBy: 'state',
      span: 1,
    },
    {
      id: 'health-blocked',
      title: 'Blocked',
      help: 'Open tasks waiting on something else. These are the ones costing time silently.',
      viz: 'number',
      filter: { state: OPEN, is_blocked: 'true' },
      groupBy: 'state',
      span: 1,
    },
    {
      id: 'health-throughput',
      title: 'Throughput per week',
      help: 'Tasks reaching a completed state each week. Cancelled work is not counted.',
      viz: 'line',
      groupBy: 'state',
      measure: 'throughput',
      bucket: WEEKLY,
      span: 2,
    },
    {
      id: 'health-cycle',
      title: 'Cycle time by project (median)',
      help: 'From the moment work started to the moment it was completed. Half of the work took less than this.',
      viz: 'bar',
      groupBy: 'project',
      measure: 'cycle_time',
      span: 2,
    },
    {
      id: 'health-cycle-p90',
      title: 'Cycle time by project (90th percentile)',
      help: 'Nine in ten tasks finished faster than this. The gap from the median is how unpredictable delivery is.',
      viz: 'bar',
      groupBy: 'project',
      measure: 'p90_cycle_time',
      span: 2,
    },
    {
      id: 'health-lead',
      title: 'Lead time by project (median)',
      help: 'From the moment work was requested to the moment it was completed — the wait a requester actually feels.',
      viz: 'bar',
      groupBy: 'project',
      measure: 'lead_time',
      span: 2,
    },
    {
      id: 'health-by-state',
      title: 'Open work by state',
      help: 'Where the open work is sitting.',
      viz: 'stacked_bar',
      filter: { state: OPEN },
      groupBy: 'state',
      span: 2,
    },
    {
      id: 'health-by-type',
      title: 'Open work by type',
      help: 'How much of what is open is unplanned — bugs and incidents against everything else.',
      viz: 'bar',
      filter: { state: OPEN },
      groupBy: 'type',
      span: 2,
    },
  ],
}

const TEAM_WORKLOAD: Dashboard = {
  id: 'team-workload',
  name: 'Team workload',
  description: 'Who is carrying what, and what nobody has picked up.',
  tiles: [
    {
      id: 'load-unassigned',
      title: 'Unassigned',
      help: 'Open tasks with nobody on them.',
      viz: 'number',
      filter: { state: OPEN, assignee: '' },
      groupBy: 'state',
      span: 1,
    },
    {
      id: 'load-untriaged',
      title: 'Untriaged',
      help: 'Open tasks that no team owns yet.',
      viz: 'number',
      filter: { state: OPEN, team: '' },
      groupBy: 'state',
      span: 1,
    },
    {
      id: 'load-per-assignee',
      title: 'Open per assignee',
      help: 'Open tasks each person is carrying. Unassigned work is a row, not a gap.',
      viz: 'bar',
      filter: { state: OPEN },
      groupBy: 'assignee',
      limit: 20,
      span: 2,
    },
    {
      id: 'load-overdue-per-assignee',
      title: 'Overdue per assignee',
      help: 'Of that work, what has already missed its due date.',
      viz: 'bar',
      filter: { state: OPEN, due_at: '<@today' },
      groupBy: 'assignee',
      limit: 20,
      span: 2,
    },
    {
      id: 'load-per-team',
      title: 'Open per team',
      help: 'The same question one level up.',
      viz: 'bar',
      filter: { state: OPEN },
      groupBy: 'team',
      limit: 20,
      span: 2,
    },
  ],
}

const QUALITY: Dashboard = {
  id: 'quality',
  name: 'Quality',
  description: 'Unplanned work — what is breaking, and how long it takes to fix.',
  tiles: [
    {
      id: 'quality-open-bugs',
      title: 'Open bugs',
      help: 'Every open task of type Bug in the projects you can see.',
      viz: 'number',
      filter: { state: OPEN, type: 'BUG' },
      groupBy: 'state',
      span: 1,
    },
    {
      id: 'quality-open-incidents',
      title: 'Open incidents',
      help: 'Open incidents — the work that interrupted someone.',
      viz: 'number',
      filter: { state: OPEN, type: 'INCIDENT' },
      groupBy: 'state',
      span: 1,
    },
    {
      id: 'quality-bugs-by-priority',
      title: 'Open bugs by priority',
      help: 'Whether the backlog of defects is urgent or merely long.',
      viz: 'bar',
      filter: { state: OPEN, type: 'BUG' },
      groupBy: 'priority',
      span: 2,
    },
    {
      id: 'quality-bug-cycle',
      title: 'Bug cycle time by priority (median)',
      help: 'How long fixing a bug takes once it is started. If urgent is not faster than low, priority is not doing anything.',
      viz: 'bar',
      filter: { type: 'BUG' },
      groupBy: 'priority',
      measure: 'cycle_time',
      span: 2,
    },
    {
      id: 'quality-bug-throughput',
      title: 'Bugs closed per week',
      help: 'Bugs reaching a completed state each week. Cancelled bugs are not counted as fixed.',
      viz: 'line',
      filter: { type: 'BUG' },
      groupBy: 'state',
      measure: 'throughput',
      bucket: WEEKLY,
      span: 2,
    },
    {
      id: 'quality-unplanned-by-team',
      title: 'Open unplanned work by team',
      help: 'Bugs and incidents by the team that owns them.',
      viz: 'bar',
      filter: { state: OPEN, type: 'BUG,INCIDENT' },
      groupBy: 'team',
      limit: 20,
      span: 2,
    },
  ],
}

export const DASHBOARDS: readonly Dashboard[] = [
  MY_WORK,
  PROJECT_HEALTH,
  TEAM_WORKLOAD,
  QUALITY,
] as const

export function dashboardById(id: string): Dashboard | undefined {
  return DASHBOARDS.find((dashboard) => dashboard.id === id)
}

/**
 * ## Not built
 *
 * Four tiles `docs/38` names are absent, because the measure set cannot express
 * them and approximating a metric is worse than omitting it — a wrong number on
 * a dashboard gets quoted in a meeting, where a missing one gets asked about.
 *
 * | Tile | Needs | Why it cannot be faked |
 * | --- | --- | --- |
 * | Created vs completed | `created_vs_completed` (two series) | Two separate runs would be two scopes and two cache windows; the crossing point is the whole message. |
 * | Age of oldest open | `age` | `updated_at` is not age, and `created_at` ordering is a list query, not an aggregate. |
 * | Reopen rate | — | Needs completed→active transitions counted, which `task_state_interval` records but no measure exposes. |
 * | Time in state | `time_in_state` | The server refuses it by name (`TF-SYS-0007`). |
 *
 * These are tracked as D-items rather than left as a comment nobody finds.
 */
