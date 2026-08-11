/**
 * A dashboard — a named layout of reports (`docs/38` §Dashboards).
 *
 * # Why a dashboard is not "Reports with more charts"
 *
 * They answer different questions and so they are shaped differently. Reports
 * is *interrogative*: you arrive with a question, drive the toolbar, and read
 * one number at a time. A dashboard is *declarative*: it answers the four or
 * five questions you would have asked anyway, at a glance, without being
 * driven. That is why there is no filter toolbar here — a dashboard you have to
 * configure before it says anything is a report builder with extra steps, and
 * `docs/01`'s simplicity contract already refused to add one.
 *
 * The two are joined rather than duplicated: every tile is a report, expressed
 * in the same four-part model, and "Open in reports" would be a link that
 * hands the tile's own filter to the toolbar.
 *
 * # Why the tiles are a grid of independent sections
 *
 * Each is an `h3` inside a labelled `section`, so a screen-reader user can list
 * the tiles and jump between them, and each carries a hidden data table with the
 * numbers its chart draws. `docs/47`'s contract: a chart is decoration, the
 * table is the content.
 */
import type { ReactElement } from 'react'
import { Link } from '@tanstack/react-router'

import { PageHeader } from '../../shell/PageHeader'
import { useAppSearch } from '../../shell/navigation'
import { useWorkspaceId } from '../../shell/session'
import { useVocabulary } from '../reports/vocabulary'
import { breakdownsOf, DASHBOARDS, dashboardById, signalsOf } from './builtin'
import { Tile } from './Tile'

// Imported here rather than from `app.css` so it ships in this route's own
// chunk. ADR-024 budgets the *initial* shell, and nobody who never opens a
// dashboard should download the chart styles.
import './dashboard.css'

export default function DashboardView({ id }: { id: string }): ReactElement {
  const workspaceId = useWorkspaceId()
  const search = useAppSearch()
  const dashboard = dashboardById(id)

  // The project scope is the sidebar's, not this page's. Every other view
  // narrows to it, and a dashboard that ignored it would report workspace
  // totals under a heading that says one project — the kind of wrong that gets
  // believed because nothing looks broken.
  const scope = search.project === undefined ? {} : { project: search.project }
  const signals = dashboard === undefined ? [] : signalsOf(dashboard)
  const breakdowns = dashboard === undefined ? [] : breakdownsOf(dashboard)

  // Every dimension this dashboard slices by, resolved once for all its tiles
  // rather than once per tile. Twelve tiles grouped by assignee share a single
  // member request through the query cache.
  const { label } = useVocabulary(workspaceId, dashboard?.tiles.map((t) => t.groupBy) ?? [])

  if (dashboard === undefined) {
    return (
      <section className="view" aria-labelledby="page-title">
        <PageHeader title="Dashboard" />
        <div className="view__body">
          <p className="empty">There is no dashboard by that name.</p>
        </div>
      </section>
    )
  }

  return (
    <section className="view" aria-labelledby="page-title">
      <PageHeader title={dashboard.name} />

      <nav className="dash__tabs" aria-label="Dashboards">
        <ul>
          {DASHBOARDS.map((option) => (
            <li key={option.id}>
              <Link
                to="/dashboards/$id"
                params={{ id: option.id }}
                className="dash__tab"
                activeProps={{ className: 'dash__tab dash__tab--on', 'aria-current': 'page' }}
              >
                {option.name}
              </Link>
            </li>
          ))}
        </ul>
      </nav>

      <div className="view__body dash">
        {/* The signals first, and visually louder than everything under them.
            A dashboard where the exception and the background look alike is a
            dashboard nobody scans — they read it, once, and then stop opening
            it. These are the numbers that can demand action; the charts below
            are the explanation you go to *after* one of them catches your eye. */}
        <section className="dash__signals" aria-label="Signals">
          {signals.map((tile) => (
            <Tile key={tile.id} spec={tile} label={label} scope={scope} />
          ))}
        </section>

        <div className="dash__grid">
          {breakdowns.map((tile) => (
            <Tile key={tile.id} spec={tile} label={label} scope={scope} />
          ))}
        </div>

        {/* Said on the page rather than in a tooltip, for the reason `docs/38`
            gives: the permission filter is injected per viewer, so two people
            reading this dashboard see different totals and both are right. A
            number quoted without that is a number someone will argue about. */}
        <p className="dash__scope">
          Every number here covers only the projects you can see. Someone with different access
          will read different totals from the same dashboard.
        </p>
      </div>
    </section>
  )
}
