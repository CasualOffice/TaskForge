/**
 * What a surface shows when it has nothing to show.
 *
 * # The failure this module prevents
 *
 * Blank space, or the word "None". `design/LAYOUT-AND-INTERACTION-GUIDELINES.md`
 * §10: "Empty states are operational." An empty column that says nothing is
 * indistinguishable from a column that failed to load, and the user's next
 * move — change the filter, create a task, pick a project — is exactly the
 * thing the screen is best placed to tell them.
 *
 * # Why the copy is a prop and the shape is not
 *
 * VISUAL-IDENTITY §9 fixes the voice: "concise, factual and operational",
 * against "cute empty-state copy" and "vague enterprise language". A component
 * cannot enforce tone, but it can enforce *structure* — a statement of fact,
 * then what to do about it — so that a caller who has only a sentence is
 * prompted for the action, and one with no action states the fact plainly
 * rather than padding it.
 *
 * # Two tiers
 *
 * `docs/54` gives a full blank slate a bounded explanatory illustration and a
 * compact panel a contextual glyph. The component keeps them separate so a
 * board column cannot accidentally pull a large scene into dense work.
 */
import type { ReactElement, ReactNode } from 'react'

export function EmptyState({
  title,
  detail,
  actions,
  icon,
  illustration,
  compact = false,
}: {
  /** The fact, as a sentence. "No tasks match this view." */
  title: string
  /** Why, when the reason is not obvious from the title. */
  detail?: ReactNode
  /** What to do next. Buttons or links; omitted when there is genuinely nothing. */
  actions?: ReactNode
  icon?: ReactElement
  /** Original, aria-hidden SVG for a full blank slate. Ignored in compact mode. */
  illustration?: ReactElement
  /** For a board column or another small container, where the full block would
   *  be taller than the space it sits in. */
  compact?: boolean
}): ReactElement {
  return (
    <div className={compact ? 'empty-state empty-state--compact' : 'empty-state'}>
      {!compact && illustration !== undefined ? (
        <div className="empty-state__illustration" aria-hidden="true">
          {illustration}
        </div>
      ) : null}
      {icon === undefined ? null : (
        <span className="empty-state__icon" aria-hidden="true">
          {icon}
        </span>
      )}
      <p className="empty-state__title">{title}</p>
      {detail === undefined ? null : <p className="empty-state__detail">{detail}</p>}
      {actions === undefined ? null : <div className="empty-state__actions">{actions}</div>}
    </div>
  )
}

/**
 * The shape of the content that is coming, drawn in the space it will occupy.
 *
 * §10 of the foundation asks for "skeletons for initial content whose geometry
 * is known" and §7 for stable geometry — a spinner in the middle of an empty
 * list is neither, and the content lands by pushing everything down. It lives
 * beside `EmptyState` because they are the same decision made twice: what a
 * surface shows before it can show the thing itself.
 *
 * `aria-hidden` on the bars with a sibling `role="status"`: the bars are
 * decoration, the sentence is the announcement.
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
