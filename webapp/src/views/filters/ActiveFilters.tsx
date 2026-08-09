/**
 * The filters currently applied, as removable chips.
 *
 * # The failure this module prevents
 *
 * A filter you cannot see. Once the toolbar has eight controls, several of them
 * collapsed behind a "More" popover, the only honest answer to "why is this list
 * empty?" is a line that names every constraint in force. Without it a user
 * narrows a view, forgets one control, and concludes the data is missing —
 * which is the complaint that produced this work in the first place.
 *
 * # Why each chip removes exactly one clause
 *
 * `Clear` removes everything and is the wrong tool when seven of eight
 * constraints are wanted. A chip per clause makes un-narrowing as granular as
 * narrowing was, and keeps the address bar as the single source of truth: a chip
 * writes `undefined` to one search parameter and nothing else.
 */
import type { ReactElement } from 'react'

import type { AppSearch } from '../../router'

export interface ActiveFilter {
  /** The search parameter this chip clears. */
  readonly key: keyof AppSearch
  /** What the constraint is called, e.g. "Priority". */
  readonly field: string
  /** The constraint in words, e.g. "Urgent or High". */
  readonly value: string
}

export function ActiveFilters({
  filters,
  onRemove,
  onClear,
}: {
  filters: readonly ActiveFilter[]
  onRemove: (key: keyof AppSearch) => void
  onClear: () => void
}): ReactElement | null {
  if (filters.length === 0) return null

  return (
    <div className="active" role="group" aria-label="Active filters">
      {filters.map((filter) => (
        <button
          key={String(filter.key)}
          type="button"
          className="chip"
          onClick={() => onRemove(filter.key)}
          // The chip is a button whose action is removal, so the accessible name
          // says so. "Priority: Urgent" alone would announce a filter and leave
          // a screen-reader user to guess what pressing it does.
          aria-label={`Remove filter ${filter.field}: ${filter.value}`}
        >
          <span className="chip__field">{filter.field}</span>
          <span className="chip__value">{filter.value}</span>
          <span className="chip__x" aria-hidden="true">
            ×
          </span>
        </button>
      ))}
      <button type="button" className="button button--quiet chip__clear" onClick={onClear}>
        Clear all
      </button>
    </div>
  )
}
