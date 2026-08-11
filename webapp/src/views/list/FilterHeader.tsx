/**
 * A column heading that also filters its own column.
 *
 * # Why the filter is here and not in the toolbar
 *
 * A toolbar of dropdowns makes a reader map "Status" in one place onto a column
 * somewhere else, and that mapping *is* the work — it is why a filtered list
 * reads as a list somebody else configured rather than one you narrowed. Every
 * application this one is measured against puts the control on the column it
 * narrows, and shows on the heading whether it is narrowing anything.
 *
 * # It writes the same search parameters the toolbar does
 *
 * Not a second filter mechanism: the value goes into the URL under the field's
 * own name, so a header filter, a toolbar control and a pasted link are the
 * same filter written three ways. `docs/27` §URL form is the one grammar.
 *
 * # Sorting and filtering share a heading without fighting
 *
 * The label sorts when the column is sortable; the funnel filters. Two targets,
 * one heading, and neither is a menu you have to open to discover the other.
 */
import {
  useEffect,
  useId,
  useLayoutEffect,
  useRef,
  useState,
  type CSSProperties,
  type ReactElement,
} from 'react'
import { Button, Icon } from '@schnsrw/design-system'

import type { Sort, SortKey } from '../../api/tasks'
import { useAppSearch, useUpdateSearch } from '../../shell/navigation'
import { place } from '../../shell/Popover'

export interface FilterOption {
  readonly value: string
  readonly label: string
}

export function FilterHeader({
  label,
  field,
  options,
  className = '',
  sortColumn,
  sort,
  onSort,
}: {
  label: string
  /** The search parameter this column narrows — `status`, `type`, `priority`. */
  field: 'status' | 'type' | 'priority' | 'assignee'
  options: readonly FilterOption[]
  className?: string
  /** Present when the column also sorts. */
  sortColumn?: SortKey
  sort?: Sort
  onSort?: (next: Sort) => void
}): ReactElement {
  const search = useAppSearch()
  const update = useUpdateSearch()
  const [open, setOpen] = useState(false)
  const host = useRef<HTMLDivElement>(null)
  const trigger = useRef<HTMLButtonElement>(null)
  const menu = useRef<HTMLDivElement>(null)
  const [placement, setPlacement] = useState<CSSProperties>()
  const id = useId()

  // The grammar's `in` is a comma-separated list, so the chosen set is the
  // parameter split on commas — not a second representation held beside it.
  const raw = search[field]
  const chosen = raw === undefined || raw === '' ? [] : raw.split(',')

  /**
   * Positioned in viewport coordinates, by the same function the popover uses.
   *
   * `.list` sets `overflow: hidden` — it has a rounded border and rows must not
   * escape it — so a menu positioned inside the table was clipped by it and
   * appeared *behind* the rows with nothing visible. No z-index fixes that: a
   * clipped box is not a stacking problem, it is a box the browser was told not
   * to paint outside its parent.
   *
   * `fixed` takes it out of that containing block, and reusing `place` means
   * there is one rule about where a floating surface goes, already tested
   * against both ways it can land off-screen.
   */
  useLayoutEffect(() => {
    if (!open) {
      setPlacement(undefined)
      return
    }
    const anchor = trigger.current?.getBoundingClientRect()
    const panel = menu.current?.getBoundingClientRect()
    if (anchor === undefined || panel === undefined) return
    const { left, top } = place(anchor, panel, 'start', {
      width: window.innerWidth,
      height: window.innerHeight,
    })
    setPlacement({ position: 'fixed', left, top, right: 'auto' })
  }, [open])

  useEffect(() => {
    if (!open) return
    const close = (event: MouseEvent): void => {
      if (!host.current?.contains(event.target as Node)) setOpen(false)
    }
    const escape = (event: KeyboardEvent): void => {
      if (event.key === 'Escape') setOpen(false)
    }
    document.addEventListener('mousedown', close)
    document.addEventListener('keydown', escape)
    return () => {
      document.removeEventListener('mousedown', close)
      document.removeEventListener('keydown', escape)
    }
  }, [open])

  function toggle(value: string): void {
    const next = chosen.includes(value) ? chosen.filter((one) => one !== value) : [...chosen, value]
    update({ [field]: next.length === 0 ? undefined : next.join(',') })
  }

  const sortable = sortColumn !== undefined && sort !== undefined && onSort !== undefined
  const sorted = sortable && sort.key === sortColumn

  return (
    <th
      scope="col"
      className={`list__cell colhead ${className}`}
      aria-sort={sorted ? (sort.descending ? 'descending' : 'ascending') : undefined}
    >
      <div className="colhead__row" ref={host}>
        {sortable ? (
          <button
            type="button"
            className="list__sort colhead__label"
            onClick={() =>
              onSort({ key: sortColumn, descending: sorted ? !sort.descending : true })
            }
          >
            {label}
            {sorted ? <span aria-hidden="true">{sort.descending ? ' ↓' : ' ↑'}</span> : null}
          </button>
        ) : (
          <span className="colhead__label">{label}</span>
        )}

        <button
          type="button"
          ref={trigger}
          className={`colhead__filter${chosen.length > 0 ? ' colhead__filter--on' : ''}`}
          aria-expanded={open}
          aria-haspopup="true"
          aria-controls={open ? id : undefined}
          // The count is in the name, not only in a badge: a screen-reader user
          // is told the column is narrowed without having to open it.
          aria-label={
            chosen.length === 0
              ? `Filter by ${label.toLowerCase()}`
              : `Filter by ${label.toLowerCase()}, ${chosen.length} selected`
          }
          onClick={() => setOpen(!open)}
        >
          <Icon name="filter_list" size="sm" />
          {chosen.length > 0 ? <span className="colhead__count">{chosen.length}</span> : null}
        </button>

        {open ? (
          <div className="colhead__menu" id={id} ref={menu} style={placement}>
            <ul className="colhead__options">
              {options.map((option) => (
                <li key={option.value}>
                  <label className="colhead__option">
                    <input
                      type="checkbox"
                      checked={chosen.includes(option.value)}
                      onChange={() => toggle(option.value)}
                    />
                    <span>{option.label}</span>
                  </label>
                </li>
              ))}
            </ul>
            {chosen.length > 0 ? (
              <Button variant="subtle" size="sm" onClick={() => update({ [field]: undefined })}>
                Clear
              </Button>
            ) : null}
          </div>
        ) : null}
      </div>
    </th>
  )
}
