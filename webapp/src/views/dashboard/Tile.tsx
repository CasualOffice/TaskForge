/**
 * One tile: one report, one shape, one failure boundary — and one place to go.
 *
 * # Why the query lives here and not on the page
 *
 * `docs/38`: "Tiles load independently and lazily. One slow report degrades its
 * own tile, not the page." A page-level `useQueries` would settle together — the
 * dashboard would show nothing until its slowest tile returned, and one tile
 * erroring would take the eight good numbers down with it. So each tile owns its
 * query, its skeleton, its empty state and its error, and a dashboard is just a
 * grid of them.
 *
 * # Why the whole tile is a link
 *
 * The first version of this file rendered numbers and stopped. That is where a
 * dashboard stops being useful: someone reads "Overdue 3", believes it, and then
 * has to go and rebuild that filter by hand in the list to find out *which*
 * three. So a tile counting tasks is a link to exactly those tasks — its own
 * filter, handed to the list through `searchFromFilter`. The count and the rows
 * come from the same clause, so they cannot disagree.
 *
 * A duration tile is deliberately **not** a link: "cycle time by project" is a
 * measurement over completed work, and a list behind it would have a row count
 * with no relationship to the number above it. A link that lands somewhere
 * unrelated is worse than no link.
 */
import type { ReactElement, ReactNode } from 'react'
import { Link } from '@tanstack/react-router'
import { useQuery } from '@tanstack/react-query'

import { keys } from '../../api/keys'
import { runReport, type Dimension } from '../../api/reports'
import { ErrorNotice } from '../../shell/notice'
import { useWorkspaceId } from '../../shell/session'
import { searchFromFilter } from '../../tasks/query'
import { formatValue } from '../reports/vocabulary'
import { throttled } from './gate'
import {
  BarChart,
  DonutChart,
  LineChart,
  NumberChart,
  StackedBarChart,
  TableChart,
} from './charts'
import type { Point } from './charts'
import type { Tile as TileSpec } from './builtin'

/**
 * A bucket start, as an axis label.
 *
 * Short and locale-aware: the series axis has room for "4 Aug", not for
 * "4 August 2026", and a chart whose labels overlap communicates less than one
 * with no labels at all.
 */
const AXIS_DATE = new Intl.DateTimeFormat(undefined, { month: 'short', day: 'numeric' })

function bucketLabel(iso: string): string {
  const at = new Date(iso)
  return Number.isNaN(at.getTime()) ? iso : AXIS_DATE.format(at)
}

export function Tile({
  spec,
  label,
  scope,
}: {
  spec: TileSpec
  label: (key: string | null, dimension: Dimension) => string
  /** The project the whole dashboard is scoped to, if any. Folded into every tile. */
  scope: { readonly project?: string }
}): ReactElement {
  const workspaceId = useWorkspaceId()

  // The dashboard's scope is part of every tile's question, not a decoration on
  // the page around it. A project chosen in the sidebar that did not reach the
  // numbers would make the whole surface quietly wrong.
  const filter = { ...spec.filter, ...(scope.project === undefined ? {} : { project: scope.project }) }
  const hasFilter = Object.keys(filter).length > 0

  const report = useQuery({
    queryKey: keys.report(workspaceId, {
      tile: spec.id,
      filter,
      slice: spec.groupBy,
      measure: spec.measure ?? 'count',
      bucket: spec.bucket,
    }),
    queryFn: ({ signal }) =>
      // Queued rather than fired: nine tiles mounting together would otherwise
      // send nine of the most expensive queries in the product at once. See
      // `gate.ts` for why the browser bounds this and not only the edge.
      throttled(
        () =>
          runReport(
            workspaceId,
            {
              groupBy: spec.groupBy,
              ...(hasFilter ? { filter } : {}),
              ...(spec.measure === undefined ? {} : { measure: spec.measure }),
              ...(spec.bucket === undefined ? {} : { bucket: spec.bucket }),
              ...(spec.limit === undefined ? {} : { limit: spec.limit }),
            },
            signal,
          ),
        signal,
      ),
    enabled: workspaceId !== '',
    // `docs/38` §Report execution limits: results cache for 5 minutes, and a
    // dashboard left open on a wall display must not become a load generator.
    staleTime: 5 * 60_000,
  })

  const unit = report.data?.unit ?? 'tasks'
  const groups = report.data?.groups ?? []
  const total = report.data?.total ?? 0

  const points: readonly Point[] = spec.bucket
    ? // A series is keyed by bucket, not by slice: the group key is constant
      // (throughput is always `COMPLETED`) and it is the time axis that varies.
      groups.map((group) => ({
        label: bucketLabel(group.bucket_start ?? ''),
        value: group.total,
        formatted: formatValue(group.total, unit),
      }))
    : groups.map((group) => ({
        label: label(group.key, spec.groupBy),
        value: group.total,
        formatted: formatValue(group.total, unit),
      }))

  // Loud only when there is something to be loud about. Zero overdue is the
  // answer someone came to see, and colouring it red for its category would
  // train people to stop reading the colour.
  const tone = spec.intent === undefined || total === 0 ? 'calm' : spec.intent

  const body = report.isPending ? (
    <p className="tile__pending" role="status">
      Loading…
    </p>
  ) : report.error ? (
    <ErrorNotice error={report.error} />
  ) : spec.viz === 'number' ? (
    <NumberChart value={formatValue(total, unit)} unit={unit === 'seconds' ? '' : 'tasks'} />
  ) : points.length === 0 ? (
    <p className="tile__empty">Nothing to show yet.</p>
  ) : (
    <Chart spec={spec} points={points} unit={unit} />
  )

  const inner = (
    <>
      <header className="tile__head">
        <h3 className="tile__title" id={`tile-${spec.id}`}>
          {spec.title}
        </h3>
        {spec.help === undefined ? null : <p className="tile__help">{spec.help}</p>}
      </header>
      <div className="tile__body">{body}</div>
    </>
  )

  const className = `tile tile--${spec.viz} tile--span${spec.span} tile--${tone}`

  // A tile that cannot be opened is a `section`, not a dead link. The failure
  // this avoids is a card that looks clickable everywhere and does nothing on
  // four of them.
  if (!spec.drillable) {
    return (
      <section className={className} aria-labelledby={`tile-${spec.id}`}>
        {inner}
      </section>
    )
  }

  return (
    <Link
      to="/"
      search={searchFromFilter(filter)}
      className={`${className} tile--open`}
      aria-labelledby={`tile-${spec.id}`}
    >
      {inner}
      {/* Named for a screen reader, hidden from the eye: the arrow says
          "openable" visually, and this says where it opens to. */}
      <span className="visually-hidden">Open these tasks in the list</span>
      <span className="tile__go material-symbols-outlined" aria-hidden="true">
        arrow_forward
      </span>
    </Link>
  )
}

function Chart({
  spec,
  points,
  unit,
}: {
  spec: TileSpec
  points: readonly Point[]
  unit: string
}): ReactNode {
  const caption = `${spec.title}${spec.help === undefined ? '' : `. ${spec.help}`}`
  const dimension = spec.bucket ? 'Week beginning' : DIMENSION_LABEL[spec.groupBy]
  const measure = unit === 'seconds' ? 'Duration' : 'Tasks'
  const args = { points, caption, dimension, measure }

  switch (spec.viz) {
    case 'line':
      return <LineChart {...args} />
    case 'donut':
      return <DonutChart {...args} />
    case 'stacked_bar':
      return <StackedBarChart {...args} />
    case 'table':
      return <TableChart {...args} />
    case 'bar':
    case 'number':
      return <BarChart {...args} />
  }
}

/** The column heading a slice gets in the table every chart carries. */
const DIMENSION_LABEL: Record<Dimension, string> = {
  status: 'Status',
  state: 'State',
  type: 'Type',
  priority: 'Priority',
  project: 'Project',
  team: 'Team',
  environment: 'Environment',
  reporter: 'Reporter',
  milestone: 'Milestone',
  assignee: 'Assignee',
}
