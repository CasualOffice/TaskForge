/**
 * The built-in dashboards (`docs/38` §Built-in dashboards).
 *
 * # What a dashboard is for, which the first version of this file forgot
 *
 * It is not a page of reports. It answers one question — **is anything wrong,
 * and where do I go about it** — in about two seconds, without being driven.
 * The first version failed that in three ways worth naming, because each one
 * has a rule here now:
 *
 * 1. **Every tile weighed the same.** Nine identical cards is nine things to
 *    read and nothing to notice. Tiles now split into *signals* — the handful of
 *    numbers that can demand action — and the breakdowns that explain them.
 * 2. **Nothing had a state.** "Overdue 3" and "Open 47" were the same colour, so
 *    the screen never said which was the problem. Each signal declares an
 *    [`Intent`], and a signal at zero reads calm while the same tile at three
 *    reads loud.
 * 3. **Nothing was clickable.** A number you cannot act on is trivia. Every tile
 *    carries a `filter`, and that filter is now also a link to the list showing
 *    exactly those tasks — so the dashboard is the start of a workflow rather
 *    than the end of one.
 *
 * # Why these are data and not components
 *
 * `docs/38`: "Shipped, expressed entirely in the model above — which is the
 * proof the model is sufficient. If a built-in dashboard needed a capability the
 * model lacks, that is the signal the model is under-specified."
 *
 * So every tile is a `filter` + `measure` + `group_by` + `bucket` and nothing
 * else, sent to the same endpoint, with no private path a hand-built tile could
 * take. When saved reports arrive, these become rows rather than a rewrite.
 *
 * # Why the filters are strings
 *
 * They are the URL form from `docs/27` — `state=BACKLOG,PLANNED,ACTIVE`,
 * `due_at=<@today`, `assignee=` for is-empty. The same grammar the address bar
 * carries, which is what lets a tile become a link at all: `searchFromFilter`
 * turns the tile's own filter into the list's address. `@me` and `@today` are
 * resolved by the server against the reader and their timezone rather than
 * baked in here against the browser's.
 */
import type { TaskFilter } from '../../api/tasks'
import type { Bucket, Dimension, MeasureKey } from '../../api/reports'

/**
 * The visualizations, closed (`docs/38`).
 *
 * `donut` is an addition to the set the doc originally closed, made on request
 * and recorded there rather than left as a divergence: composition is a
 * question a stacked bar answers badly. A bar is read as a length and invites
 * comparing segments to each other; a ring is read as a proportion and answers
 * "how much of the work is this". `heatmap` remains unbuilt — nothing needs one
 * yet, and a menu is a promise.
 */
export type Viz = 'number' | 'bar' | 'line' | 'donut' | 'stacked_bar' | 'table'

/**
 * What a number *means* when it is not zero.
 *
 * Deliberately three values and no thresholds. A dashboard that decided 20 open
 * bugs was amber and 21 was red would be inventing a policy no workspace agreed
 * to, and would be wrong for every team with a different size. What the product
 * can say honestly is the *kind* of number this is:
 *
 * - `alert` — this represents a commitment already missed. Overdue work is the
 *   only honest member: the date has passed, and no team considers that fine.
 * - `watch` — this is work that has stalled or that nobody has picked up. Not a
 *   failure, but the reason to open the dashboard.
 * - `plain` — informational. Big is not bad; it is just the size of the work.
 *
 * Zero renders calm whatever the intent, because zero overdue is the answer
 * someone came to see and colouring it red for its category would train people
 * to ignore the colour.
 */
export type Intent = 'alert' | 'watch' | 'plain'

export interface Tile {
  readonly id: string
  readonly title: string
  /**
   * One short line, and only where the measure is not self-defining.
   *
   * "Overdue" needs no gloss; "Cycle time" does. Prose on every tile is what
   * made the first version read like a page of text rather than a dashboard,
   * so this is optional and stays to a single line.
   */
  readonly help?: string
  readonly viz: Viz
  /** Signals only. Charts are always `plain` — a distribution is not a verdict. */
  readonly intent?: Intent
  readonly filter?: TaskFilter
  readonly groupBy: Dimension
  readonly measure?: MeasureKey
  readonly bucket?: Bucket
  readonly limit?: number
  /**
   * Whether this tile can be opened as a list of tasks.
   *
   * True for anything counting tasks. **False for a duration**: "cycle time by
   * project" is a measurement over completed work, and a link from it would
   * show a list of tasks whose row count has nothing to do with the number
   * above it. A link that lands somewhere unrelated is worse than no link.
   */
  readonly drillable: boolean
  /** Columns of the 4-wide grid. Signals are 1; a series wants 2. */
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
 * Defined once because "open" appears in most tiles and a dashboard whose tiles
 * disagreed about whether cancelled work is open would be worse than one with
 * no numbers at all. `CANCELED` is excluded for the reason `docs/38` gives:
 * collapsing cancelled into completed is the most common metric bug in
 * trackers, and it is why the state exists separately.
 */
const OPEN = 'BACKLOG,PLANNED,ACTIVE'

/**
 * Throughput needs an interval but ignores the field.
 *
 * `compile_throughput` buckets on `task_state_interval.entered_at` — when the
 * work actually reached `COMPLETED` — because there is no `completed_at` column
 * to truncate. The request shape still requires a field, so this names the one
 * that is least misleading if anyone reads the payload, and says here that it is
 * not what the series is bucketed by.
 */
const WEEKLY: Bucket = { field: 'created_at', interval: 'week' }

const MY_WORK: Dashboard = {
  id: 'my-work',
  name: 'My work',
  description: 'What is on you right now.',
  tiles: [
    {
      id: 'mine-overdue',
      title: 'Overdue',
      viz: 'number',
      intent: 'alert',
      filter: { assignee: '@me', state: OPEN, due_at: '<@today' },
      groupBy: 'state',
      drillable: true,
      span: 1,
    },
    {
      id: 'mine-due-soon',
      title: 'Due this week',
      viz: 'number',
      intent: 'watch',
      filter: { assignee: '@me', state: OPEN, due_at: '@today..+7d' },
      groupBy: 'state',
      drillable: true,
      span: 1,
    },
    {
      id: 'mine-blocked',
      title: 'Blocked',
      viz: 'number',
      intent: 'watch',
      filter: { assignee: '@me', state: OPEN, is_blocked: 'true' },
      groupBy: 'state',
      drillable: true,
      span: 1,
    },
    {
      id: 'mine-open',
      title: 'Assigned to you',
      viz: 'number',
      intent: 'plain',
      filter: { assignee: '@me', state: OPEN },
      groupBy: 'state',
      drillable: true,
      span: 1,
    },
    {
      id: 'mine-by-state',
      title: 'Where your work sits',
      viz: 'donut',
      filter: { assignee: '@me', state: OPEN },
      groupBy: 'state',
      drillable: true,
      span: 2,
    },
    {
      id: 'mine-by-priority',
      title: 'By priority',
      viz: 'bar',
      filter: { assignee: '@me', state: OPEN },
      groupBy: 'priority',
      drillable: true,
      span: 2,
    },
    {
      id: 'mine-throughput',
      title: 'Completed per week',
      help: 'Counted when work reached a completed state. Cancelled work is not counted.',
      viz: 'line',
      filter: { assignee: '@me' },
      groupBy: 'state',
      measure: 'throughput',
      bucket: WEEKLY,
      drillable: false,
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
      id: 'health-overdue',
      title: 'Overdue',
      viz: 'number',
      intent: 'alert',
      filter: { state: OPEN, due_at: '<@today' },
      groupBy: 'state',
      drillable: true,
      span: 1,
    },
    {
      id: 'health-blocked',
      title: 'Blocked',
      viz: 'number',
      intent: 'watch',
      filter: { state: OPEN, is_blocked: 'true' },
      groupBy: 'state',
      drillable: true,
      span: 1,
    },
    {
      id: 'health-unassigned',
      title: 'Unassigned',
      viz: 'number',
      intent: 'watch',
      filter: { state: OPEN, assignee: '' },
      groupBy: 'state',
      drillable: true,
      span: 1,
    },
    {
      id: 'health-open',
      title: 'Open',
      viz: 'number',
      intent: 'plain',
      filter: { state: OPEN },
      groupBy: 'state',
      drillable: true,
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
      drillable: false,
      span: 2,
    },
    {
      id: 'health-by-state',
      title: 'Open work by state',
      viz: 'donut',
      filter: { state: OPEN },
      groupBy: 'state',
      drillable: true,
      span: 2,
    },
    {
      id: 'health-oldest',
      title: 'Oldest open work by project',
      help: 'How long the longest-waiting open task has been waiting. Finished work is not counted.',
      viz: 'bar',
      groupBy: 'project',
      measure: 'age',
      drillable: false,
      span: 2,
    },
    {
      id: 'health-cycle',
      title: 'Cycle time by project',
      help: 'Median time from work starting to work finishing.',
      viz: 'bar',
      groupBy: 'project',
      measure: 'cycle_time',
      drillable: false,
      span: 2,
    },
    {
      id: 'health-lead',
      title: 'Lead time by project',
      help: 'Median time from the request arriving to work finishing — the wait a requester feels.',
      viz: 'bar',
      groupBy: 'project',
      measure: 'lead_time',
      drillable: false,
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
      viz: 'number',
      intent: 'watch',
      filter: { state: OPEN, assignee: '' },
      groupBy: 'state',
      drillable: true,
      span: 1,
    },
    {
      id: 'load-untriaged',
      title: 'Untriaged',
      viz: 'number',
      intent: 'watch',
      filter: { state: OPEN, team: '' },
      groupBy: 'state',
      drillable: true,
      span: 1,
    },
    {
      id: 'load-overdue',
      title: 'Overdue',
      viz: 'number',
      intent: 'alert',
      filter: { state: OPEN, due_at: '<@today' },
      groupBy: 'state',
      drillable: true,
      span: 1,
    },
    {
      id: 'load-open',
      title: 'Open',
      viz: 'number',
      intent: 'plain',
      filter: { state: OPEN },
      groupBy: 'state',
      drillable: true,
      span: 1,
    },
    {
      id: 'load-per-assignee',
      title: 'Open per assignee',
      viz: 'bar',
      filter: { state: OPEN },
      groupBy: 'assignee',
      limit: 20,
      drillable: true,
      span: 2,
    },
    {
      id: 'load-overdue-per-assignee',
      title: 'Overdue per assignee',
      viz: 'bar',
      filter: { state: OPEN, due_at: '<@today' },
      groupBy: 'assignee',
      limit: 20,
      drillable: true,
      span: 2,
    },
    {
      id: 'load-per-team',
      title: 'Open per team',
      viz: 'bar',
      filter: { state: OPEN },
      groupBy: 'team',
      limit: 20,
      drillable: true,
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
      id: 'quality-open-incidents',
      title: 'Open incidents',
      viz: 'number',
      intent: 'alert',
      filter: { state: OPEN, type: 'INCIDENT' },
      groupBy: 'state',
      drillable: true,
      span: 1,
    },
    {
      id: 'quality-urgent-bugs',
      title: 'Urgent bugs',
      viz: 'number',
      intent: 'alert',
      filter: { state: OPEN, type: 'BUG', priority: 'URGENT' },
      groupBy: 'state',
      drillable: true,
      span: 1,
    },
    {
      id: 'quality-blocked-bugs',
      title: 'Blocked bugs',
      viz: 'number',
      intent: 'watch',
      filter: { state: OPEN, type: 'BUG', is_blocked: 'true' },
      groupBy: 'state',
      drillable: true,
      span: 1,
    },
    {
      id: 'quality-open-bugs',
      title: 'Open bugs',
      viz: 'number',
      intent: 'plain',
      filter: { state: OPEN, type: 'BUG' },
      groupBy: 'state',
      drillable: true,
      span: 1,
    },
    {
      id: 'quality-bug-throughput',
      title: 'Bugs closed per week',
      help: 'Cancelled bugs are not counted as fixed.',
      viz: 'line',
      filter: { type: 'BUG' },
      groupBy: 'state',
      measure: 'throughput',
      bucket: WEEKLY,
      drillable: false,
      span: 2,
    },
    {
      id: 'quality-bugs-by-priority',
      title: 'Open bugs by priority',
      viz: 'bar',
      filter: { state: OPEN, type: 'BUG' },
      groupBy: 'priority',
      drillable: true,
      span: 2,
    },
    {
      id: 'quality-bug-cycle',
      title: 'Bug cycle time by priority',
      help: 'Median time to fix. If urgent is not faster than low, priority is not doing anything.',
      viz: 'bar',
      filter: { type: 'BUG' },
      groupBy: 'priority',
      measure: 'cycle_time',
      drillable: false,
      span: 2,
    },
    {
      id: 'quality-unplanned-by-team',
      title: 'Open bugs and incidents by team',
      viz: 'bar',
      filter: { state: OPEN, type: 'BUG,INCIDENT' },
      groupBy: 'team',
      limit: 20,
      drillable: true,
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
 * The at-a-glance row: the numbers that can demand action.
 *
 * Derived from the visualization rather than declared separately, because that
 * is what the distinction *is* — a `number` is a verdict, a chart is an
 * explanation. Two lists to keep in step would be a way for a tile to be a
 * signal on one line and a breakdown on another.
 */
export function signalsOf(dashboard: Dashboard): readonly Tile[] {
  return dashboard.tiles.filter((tile) => tile.viz === 'number')
}

export function breakdownsOf(dashboard: Dashboard): readonly Tile[] {
  return dashboard.tiles.filter((tile) => tile.viz !== 'number')
}

/**
 * ## Not built
 *
 * Three tiles `docs/38` names are absent, because the measure set cannot express
 * them and approximating a metric is worse than omitting it — a wrong number on
 * a dashboard gets quoted in a meeting, where a missing one gets asked about.
 *
 * | Tile | Needs | Why it cannot be faked |
 * | --- | --- | --- |
 * | Created vs completed | `created_vs_completed` (two series) | Two separate runs would be two scopes and two cache windows; the crossing point is the whole message. |
 * | Reopen rate | — | Needs completed→active transitions counted, which `task_state_interval` records but no measure exposes. |
 * | Time in state | `time_in_state` | The server refuses it by name (`TF-SYS-0007`). |
 *
 * A **change indicator** on each signal — "3 more than last week" — is the other
 * obvious gap, and it is not built because it doubles the query count on a
 * surface that already had to be given a concurrency bound. It wants a measure
 * that returns two periods in one answer, not two runs.
 */
