/**
 * Empty, loading and failure as first-class states.
 *
 * # The failure this module prevents
 *
 * Blank space where a state should be. `design/DESIGN-FOUNDATION.md` §10:
 * "Empty, error, offline and permission-denied states are first-class product
 * states", and §11 forbids the full-screen loader that usually stands in for
 * them. `design/LAYOUT-AND-INTERACTION-GUIDELINES.md` §10 goes further and says
 * what an empty state is *for*: "Empty states are operational … No tasks match
 * this view. Change the filters or create a task." A sentence, and the control
 * that acts on it — not an illustration and not encouragement.
 *
 * # Why the action is a slot and not a prop pair
 *
 * The thing to do next is a real control with its own permission check and its
 * own mutation — "New task" is `CreateTask`, not a label and a callback. Passing
 * `{ actionLabel, onAction }` would force every caller to rebuild a button that
 * already exists somewhere better.
 */
import type { ReactElement, ReactNode } from 'react'

export function EmptyState({
  title,
  detail,
  children,
}: {
  /** One line, sentence case, describing the state. Not a heading for a page. */
  title: string
  /** What to do about it, when that is not obvious from the actions. */
  detail?: string
  /** The controls that resolve it. */
  children?: ReactNode
}): ReactElement {
  return (
    <div className="state">
      <p className="state__title">{title}</p>
      {detail === undefined ? null : <p className="state__detail">{detail}</p>}
      {children === undefined ? null : <div className="state__actions">{children}</div>}
    </div>
  )
}

/**
 * The shape of the content that is coming, drawn in the space it will occupy.
 *
 * §10 asks for "skeletons for initial content whose geometry is known" and §7
 * for stable geometry — a spinner in the middle of an empty list is neither, and
 * the content lands by pushing everything down. `aria-hidden` with a sibling
 * `role="status"`: the bars are decoration, the sentence is the announcement.
 */
export function Skeleton({ rows = 6, label }: { rows?: number; label: string }): ReactElement {
  return (
    <div className="skeleton">
      <span className="visually-hidden" role="status">
        {label}
      </span>
      <div aria-hidden="true">
        {Array.from({ length: rows }, (_, index) => (
          <div key={index} className="skeleton__row" />
        ))}
      </div>
    </div>
  )
}
