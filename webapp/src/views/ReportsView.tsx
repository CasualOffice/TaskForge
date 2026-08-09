/**
 * Reports — lazy, always.
 *
 * docs/42 §What is not — lazy, always: "Calendar · timeline/Gantt · reports and
 * charts · all admin and settings screens..." This module exists as a real route
 * behind a real `import()`, so the bundle report has a lazy chunk to attribute
 * and the gate can prove initial and lazy bytes are actually separated.
 *
 * ADR-027 fixes what a report *is* — a saved filter plus a closed measure set —
 * and `docs/38` designs the surface. Neither is built (C-021+), so this states
 * that rather than rendering an empty chart.
 */
import type { ReactElement } from 'react'
import { Link } from '@tanstack/react-router'

import { EmptyState } from '../shell/EmptyState'
import { ReportsIllustration } from '../shell/illustrations'

export default function ReportsView(): ReactElement {
  return (
    <section className="view reports-page" aria-labelledby="reports-heading">
      <header className="work-page-header">
        <div>
          <p className="work-page-header__eyebrow">Insights</p>
          <h1 id="reports-heading">Reports</h1>
          <p>Turn saved views into focused measures without crowding daily work.</p>
        </div>
      </header>
      <div className="view__body reports-page__body">
        <div className="reports-page__empty">
          <EmptyState
            illustration={<ReportsIllustration />}
            title="Reporting is being prepared."
            detail="Your work remains available in Tasks and My Work while this capability is completed."
            actions={<Link to="/" className="button button--primary">Review tasks</Link>}
          />
        </div>
      </div>
    </section>
  )
}
