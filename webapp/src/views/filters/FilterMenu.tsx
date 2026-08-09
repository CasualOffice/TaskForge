/**
 * A multi-select filter, as a button that opens a list of checkboxes.
 *
 * # Why not `<select multiple>`
 *
 * It is the cheapest thing that technically works and it is close to unusable: a
 * fixed-height scroll box, ctrl-click to add, and a click on the wrong row wipes
 * the whole selection with no undo. People lose selections in it constantly.
 *
 * # Why not a bespoke listbox either
 *
 * A `role="listbox"` with `aria-multiselectable` has to reimplement roving
 * focus, type-ahead, and selection announcement — three things a browser already
 * does for a checkbox, and the third is where hand-rolled widgets usually fail
 * silently. So the popover holds ordinary `<input type="checkbox">` elements
 * with real `<label>`s: Tab moves, Space toggles, the screen reader says
 * "checked", and none of that is this file's code.
 *
 * What this file *does* own is the part a checkbox list does not give for free:
 * the popover closes on `Escape` and on a click outside, and the trigger reports
 * how many are selected so the state is legible without opening it.
 *
 * # This presentation is temporary, and shaped to be replaced
 *
 * AGENTS.md makes `@schnsrw/design-system` a consumed dependency, and its `Menu`
 * already takes `{ label, checked, onClick }` entries — which is precisely a
 * multi-select menu. The package is not resolvable in this checkout yet, so
 * rather than hand-roll a second set of primitives that would only be deleted,
 * the options below are built as an [`items`] array in exactly that shape. When
 * the dependency lands, the `<ul>` becomes `<Menu items={items} />` and nothing
 * else in this file moves.
 */
import { useEffect, useId, useRef, useState, type ReactElement } from 'react'

export interface FilterOption {
  readonly value: string
  readonly label: string
}

export function FilterMenu({
  label,
  options,
  selected,
  onChange,
}: {
  label: string
  options: readonly FilterOption[]
  /** The comma-separated value from the URL, or `undefined` for "no constraint". */
  selected: string | undefined
  onChange: (next: string | undefined) => void
}): ReactElement {
  const [open, setOpen] = useState(false)
  const host = useRef<HTMLDivElement>(null)
  const id = useId()

  const chosen = selected === undefined || selected === '' ? [] : selected.split(',')

  useEffect(() => {
    if (!open) return
    function onDocument(event: MouseEvent): void {
      if (!host.current?.contains(event.target as Node)) setOpen(false)
    }
    function onKey(event: KeyboardEvent): void {
      if (event.key === 'Escape') setOpen(false)
    }
    // `mousedown`, not `click`: a `click` listener fires after the checkbox has
    // already toggled and re-rendered, and the popover closes under the cursor
    // on every single selection.
    document.addEventListener('mousedown', onDocument)
    document.addEventListener('keydown', onKey)
    return () => {
      document.removeEventListener('mousedown', onDocument)
      document.removeEventListener('keydown', onKey)
    }
  }, [open])

  function toggle(value: string): void {
    const next = chosen.includes(value)
      ? chosen.filter((entry) => entry !== value)
      : [...chosen, value]
    // Empty means "no constraint", which is an absent parameter — not
    // `priority=`, which the grammar reads as `is_empty` and would return the
    // rows with no priority at all.
    onChange(next.length === 0 ? undefined : next.join(','))
  }

  // The design system's `MenuItem` shape, built here so the swap is one line.
  const items = options.map((option) => ({
    label: option.label,
    value: option.value,
    checked: chosen.includes(option.value),
    onClick: () => toggle(option.value),
  }))

  return (
    <div className="filter" ref={host}>
      <button
        type="button"
        className={`button filter__trigger${chosen.length > 0 ? ' filter__trigger--on' : ''}`}
        aria-expanded={open}
        aria-haspopup="true"
        aria-controls={`${id}-list`}
        onClick={() => setOpen(!open)}
      >
        {label}
        {chosen.length > 0 ? <span className="filter__count">{chosen.length}</span> : null}
      </button>

      {open ? (
        <div className="filter__popover" id={`${id}-list`}>
          {/* Replace with `<Menu items={items} />` when the design system is
              wired — the entries already carry its `label`/`checked`/`onClick`. */}
          <ul className="filter__options">
            {items.map((item) => (
              <li key={item.value}>
                <label className="filter__option">
                  <input type="checkbox" checked={item.checked} onChange={item.onClick} />
                  <span>{item.label}</span>
                </label>
              </li>
            ))}
          </ul>
          {chosen.length > 0 ? (
            <button
              type="button"
              className="button button--quiet filter__clear"
              onClick={() => onChange(undefined)}
            >
              Clear {label.toLowerCase()}
            </button>
          ) : null}
        </div>
      ) : null}
    </div>
  )
}

/**
 * A single-choice filter, where the options are mutually exclusive.
 *
 * A native `<select>`, because for one-of-N that is genuinely the right control
 * on every platform in `docs/18` and costs nothing. Used for due dates, where
 * "overdue" and "due within a week" are not a set a user would combine.
 */
export function FilterSelect({
  label,
  options,
  value,
  onChange,
}: {
  label: string
  options: readonly FilterOption[]
  value: string | undefined
  onChange: (next: string | undefined) => void
}): ReactElement {
  const id = useId()
  return (
    <>
      <label className="visually-hidden" htmlFor={id}>
        {label}
      </label>
      <select
        id={id}
        className={`select filter__select${value !== undefined && value !== '' ? ' filter__select--on' : ''}`}
        value={value ?? ''}
        onChange={(event) => onChange(event.target.value === '' ? undefined : event.target.value)}
      >
        {options.map((option) => (
          <option key={option.value} value={option.value}>
            {option.label}
          </option>
        ))}
      </select>
    </>
  )
}
