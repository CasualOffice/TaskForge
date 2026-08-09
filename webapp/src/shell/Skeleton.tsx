/**
 * The shape of content that has not arrived.
 *
 * # The failure this module prevents
 *
 * A layout that jumps. `design/DESIGN-FOUNDATION.md` §1.7 requires stable
 * geometry — "loading, optimistic updates and live events should not cause
 * avoidable layout shifts" — and §10 says to "use skeletons for initial content
 * whose geometry is known". A centred "Loading…" followed by forty rows is the
 * shift the principle names: every pixel on the screen moves once the data
 * lands, and anything the user had begun to reach for moves with it.
 *
 * So a skeleton is not a spinner with rounded corners. It occupies the space
 * the real thing will occupy, which is why the row count and height are
 * required rather than defaulted — a caller that does not know its own geometry
 * cannot reserve it, and should show nothing instead.
 *
 * # Announced once, not forty times
 *
 * The container carries `role="status"` and a single accessible name. Marking
 * each bar would make a screen reader announce forty empty regions; marking
 * none would leave a blind user in silence while the screen visibly works.
 *
 * # Motion
 *
 * §11 lists "long skeleton transitions" as an anti-pattern and §8 keeps motion
 * to continuity, so the pulse is slow and low-contrast, and
 * `prefers-reduced-motion` removes it entirely in `tokens.css`.
 */
import type { ReactElement } from 'react'

export function SkeletonRows({
  rows,
  height,
  label = 'Loading',
}: {
  /** How many placeholder rows. Match what the surface will actually render. */
  rows: number
  /** The real row height in pixels, so nothing moves when the data arrives. */
  height: number
  label?: string
}): ReactElement {
  return (
    <div className="skeleton" role="status" aria-label={label}>
      {Array.from({ length: rows }, (_, index) => (
        <div
          key={index}
          className="skeleton__row"
          style={{ height }}
          // The bars are decoration around the one announced status above.
          aria-hidden="true"
        >
          {/* Widths vary so the block reads as text rather than as a table of
              identical grey bars — but they are derived from the index, not
              random, so a re-render cannot reshuffle them and cause the shift
              the component exists to prevent. */}
          <span className="skeleton__bar" style={{ width: `${WIDTHS[index % WIDTHS.length]}%` }} />
        </div>
      ))}
    </div>
  )
}

/** A repeating, deliberately irregular set of line lengths. */
const WIDTHS = [62, 84, 45, 73, 55, 90, 68] as const
