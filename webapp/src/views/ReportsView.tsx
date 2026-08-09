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

import { GapNotice } from '../shell/notice'

export default function ReportsView(): ReactElement {
  return (
    <section className="view" aria-labelledby="reports-heading">
      <div className="view__bar">
        <h1 id="reports-heading" className="view__title">
          Reports
        </h1>
      </div>
      <div className="view__body reports">
        {/* One line, in the reader's language. The reason this route exists
            before its contents do — it is a lazy chunk from the first commit, so
            splitting it later is not a budget regression disguised as a refactor
            — is an engineering fact and belongs in this comment, not on screen. */}
        <GapNotice what="Reports are not built yet." />
      </div>
    </section>
  )
}
