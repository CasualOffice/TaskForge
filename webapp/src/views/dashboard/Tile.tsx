/**
 * One tile: one report, one shape, one failure boundary.
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
 * That also makes the failure honest per tile: "this number is unavailable" sits
 * where the number would have been, rather than replacing the page with a
 * message that does not say which of nine reports failed.
 */
import type { ReactElement } from 'react'
import { useQuery } from '@tanstack/react-query'

import { keys } from '../../api/keys'
import { runReport, type Dimension } from '../../api/reports'
import { ErrorNotice } from '../../shell/notice'
import { useWorkspaceId } from '../../shell/session'
import { formatValue } from '../reports/vocabulary'
import { throttled } from './gate'
import { BarChart, LineChart, NumberChart, StackedBarChart, TableChart } from './charts'
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
}: {
  spec: TileSpec
  label: (key: string | null, dimension: Dimension) => string
}): ReactElement {
  const workspaceId = useWorkspaceId()

  const report = useQuery({
    queryKey: keys.report(workspaceId, {
      tile: spec.id,
      filter: spec.filter,
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
              ...(spec.filter === undefined ? {} : { filter: spec.filter }),
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

  return (
    <section
      className={`tile tile--${spec.viz} tile--span${spec.span}`}
      aria-labelledby={`tile-${spec.id}`}
    >
      <header className="tile__head">
        <h3 className="tile__title" id={`tile-${spec.id}`}>
          {spec.title}
        </h3>
        <p className="tile__help">{spec.help}</p>
      </header>

      <div className="tile__body">
        {report.isPending ? (
          <p className="tile__pending" role="status">
            Loading…
          </p>
        ) : report.error ? (
          <ErrorNotice error={report.error} />
        ) : points.length === 0 ? (
          // Zero is an answer, and for a counting tile it is usually the good
          // one — "Overdue: 0" is the number someone came to see. Only a chart
          // with no slices genuinely has nothing to draw.
          spec.viz === 'number' ? (
            <NumberChart value="0" unit={unit === 'seconds' ? '' : 'tasks'} />
          ) : (
            <p className="tile__empty">Nothing to show yet.</p>
          )
        ) : (
          <Body spec={spec} points={points} unit={unit} total={report.data?.total ?? 0} />
        )}
      </div>
    </section>
  )
}

function Body({
  spec,
  points,
  unit,
  total,
}: {
  spec: TileSpec
  points: readonly Point[]
  unit: string
  total: number
}): ReactElement {
  const caption = `${spec.title}. ${spec.help}`
  const dimension = spec.bucket ? 'Week beginning' : DIMENSION_LABEL[spec.groupBy]
  const measure = unit === 'seconds' ? 'Duration' : 'Tasks'

  switch (spec.viz) {
    case 'number':
      // The server's own total, not a sum of the groups: a report with a limit
      // returns the top N slices, and adding those up would quietly under-count
      // exactly when the number matters most.
      return <NumberChart value={formatValue(total, unit)} unit={unit === 'seconds' ? '' : 'tasks'} />
    case 'line':
      return (
        <LineChart points={points} caption={caption} dimension={dimension} measure={measure} />
      )
    case 'stacked_bar':
      return (
        <StackedBarChart points={points} caption={caption} dimension={dimension} measure={measure} />
      )
    case 'table':
      return <TableChart points={points} caption={caption} dimension={dimension} measure={measure} />
    case 'bar':
      return <BarChart points={points} caption={caption} dimension={dimension} measure={measure} />
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
