/**
 * Reports — the same filter, sliced (`docs/38`, ADR-027).
 *
 * # Why this is the work toolbar and not a report builder
 *
 * `docs/01`'s simplicity contract: reporting adds **no new user-facing noun**.
 * A report is the filter someone already knows how to write, plus a dimension
 * to slice it by. So the page is the toolbar every other view uses and one
 * extra control — "by" — and the numbers answer whatever the toolbar says. A
 * builder with its own field list would be a second grammar to learn and a
 * second one to keep in step.
 *
 * # Why the bars are CSS
 *
 * ADR-024 budgets 200 KiB for the initial shell, and `docs/42` makes the
 * charting library lazy *when there is one*. A horizontal bar is a div with a
 * width; importing a charting library to draw a rectangle would spend the
 * budget on the one chart that does not need it. When `line` and `heatmap`
 * arrive (`docs/38` §Dashboards) they bring a library with them, in their own
 * chunk.
 *
 * # Why it states its own scope
 *
 * `docs/38`: "aggregate numbers are not comparable between viewers. A manager's
 * '47 open' and a guest's '12 open' are both right." The permission filter is
 * injected per viewer, so this page says how many projects the number covers.
 * A total quoted without that is a total someone will argue about in a meeting.
 *
 * # Reports and dashboards, and which is which
 *
 * This page is *interrogative*: you arrive with a question, drive the toolbar,
 * and read one number. A dashboard is *declarative* — it answers the four
 * questions you would have asked anyway, without being driven. They share the
 * report model, the endpoint and, through `reports/vocabulary`, the words a
 * group key is read with; a slice called "Untriaged" here and "None" there
 * would be two products.
 */
import { useMemo, useState, type ReactElement } from 'react'
import { Select } from '@schnsrw/design-system'
import { useQuery } from '@tanstack/react-query'

import { keys } from '../api/keys'
import { MEASURES, runReport, type Dimension, type MeasureKey } from '../api/reports'
import { EmptyState } from '../shell/EmptyState'
import { PageHeader } from '../shell/PageHeader'
import { CONTROL } from '../shell/controls'
import { ErrorNotice } from '../shell/notice'
import { useAppSearch } from '../shell/navigation'
import { useWorkspaceId } from '../shell/session'
import { SkeletonRows } from '../shell/Skeleton'
import { filterFromSearch } from '../tasks/query'
import { WorkToolbar } from './filters/WorkToolbar'
import { duration, useVocabulary } from './reports/vocabulary'

/**
 * The slices this page can put a name to.
 *
 * Status, environment and milestone are absent on purpose: their labels live in
 * a project's own configuration, so at workspace scope the page could only
 * render uuids. A slice nobody can read is not a slice.
 */
const SLICES: readonly { key: Dimension; label: string }[] = [
  { key: 'type', label: 'Type' },
  { key: 'priority', label: 'Priority' },
  { key: 'state', label: 'State' },
  { key: 'project', label: 'Project' },
  { key: 'team', label: 'Team' },
  { key: 'assignee', label: 'Assignee' },
  { key: 'reporter', label: 'Reporter' },
]

export default function ReportsView(): ReactElement {
  const workspaceId = useWorkspaceId()
  const search = useAppSearch()
  const [slice, setSlice] = useState<Dimension>('type')
  const [measure, setMeasure] = useState<MeasureKey>('count')

  const filter = useMemo(() => filterFromSearch(search), [search])

  const report = useQuery({
    queryKey: keys.report(workspaceId, { filter, slice, measure }),
    queryFn: ({ signal }) => runReport(workspaceId, { filter, groupBy: slice, measure }, signal),
    enabled: workspaceId !== '',
  })

  // Shared with the dashboard rather than re-derived: the words a group key is
  // read with — "Untriaged", "Unassigned", "No project" — are a product
  // decision, not a formatting detail, and two copies of that decision drift.
  const { label: labelFor } = useVocabulary(workspaceId, [slice])
  const label = (key: string | null): string => labelFor(key, slice)

  const groups = report.data?.groups ?? []
  const largest = groups.reduce((most, group) => Math.max(most, group.total), 0)

  return (
    <section className="view" aria-labelledby="page-title">
      <PageHeader
        title="Reports"
        count={
          report.isPending
            ? undefined
            : report.data?.unit === 'seconds'
              ? undefined
              : `${report.data?.total ?? 0} tasks`
        }
      />
      <WorkToolbar>
        <label className="visually-hidden" htmlFor="report-measure">
          What to measure
        </label>
        <Select
          width="auto"
          containerStyle={{ maxWidth: 240 }}
          style={{ height: CONTROL }}
          id="report-measure"
          value={measure}
          onChange={(event) => setMeasure(event.target.value as MeasureKey)}
        >
          {MEASURES.map((option) => (
            <option key={option.key} value={option.key}>
              {option.label}
            </option>
          ))}
        </Select>
        <label className="visually-hidden" htmlFor="report-slice">
          Group the count by
        </label>
        <Select
          width="auto"
          containerStyle={{ maxWidth: 200 }}
          style={{ height: CONTROL }}
          id="report-slice"
          value={slice}
          onChange={(event) => setSlice(event.target.value as Dimension)}
        >
          {SLICES.map((option) => (
            <option key={option.key} value={option.key}>
              by {option.label.toLowerCase()}
            </option>
          ))}
        </Select>
      </WorkToolbar>

      <div className="view__body rep">
        {report.error ? <ErrorNotice error={report.error} /> : null}
        {report.isPending ? <SkeletonRows rows={5} height={28} label="Running the report" /> : null}

        {!report.isPending && groups.length === 0 && report.error == null ? (
          <EmptyState
            title="Nothing matches"
            detail="No task in the projects you can see matches these filters. Widen them in the bar above."
          />
        ) : null}

        {groups.length === 0 ? null : (
          <>
            <table className="rep__table">
              <caption className="visually-hidden">
                Count of tasks by {slice}, for the current filters
              </caption>
              <thead>
                <tr>
                  <th scope="col">{SLICES.find((s) => s.key === slice)?.label ?? slice}</th>
                  <th scope="col">{report.data?.unit === 'seconds' ? 'Duration' : 'Tasks'}</th>
                  <th scope="col">
                    <span className="visually-hidden">Share</span>
                  </th>
                </tr>
              </thead>
              <tbody>
                {groups.map((group) => (
                  <tr key={group.key ?? '∅'}>
                    <th scope="row" className="rep__key">
                      {label(group.key)}
                    </th>
                    <td className="rep__count">
                      {report.data?.unit === 'seconds' ? duration(group.total) : group.total}
                    </td>
                    <td className="rep__barcell">
                      {/* The bar carries no information the number does not, so
                          it is hidden from the reader who is listening rather
                          than being read out as a second, wordless column. */}
                      <div
                        className="rep__bar"
                        aria-hidden="true"
                        style={{ width: `${largest === 0 ? 0 : (group.total / largest) * 100}%` }}
                      />
                    </td>
                  </tr>
                ))}
              </tbody>
            </table>

            {/* Said on the page, not in a tooltip. Two people reading this see
                different totals and both are right, and the only way that is
                not a bug report is if the page says whose view produced it. */}
            <p className="rep__scope">
              Counted over {report.data?.scope.projects ?? 0}{' '}
              {report.data?.scope.projects === 1 ? 'project' : 'projects'} you can see. Someone with
              different access will see different numbers.
            </p>
          </>
        )}
      </div>
    </section>
  )
}
