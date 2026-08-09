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
 * silently.
 *
 * # The list is the design system's Menu
 *
 * Its entries take `{ label, checked, onClick }`, which is precisely what a
 * multi-select menu is, so the options are built in that shape and handed over
 * whole. "Clear" is an entry in the same menu rather than a button parked
 * beneath it: it acts on the selection, so it belongs where the selection is.
 *
 * What this file owns is the part the Menu does not: the popover closes on
 * `Escape` and on a click outside, and the trigger reports how many are
 * selected so the state is legible without opening it.
 */
import { Button, Menu, Select, type MenuEntry } from '@schnsrw/design-system'
import { useEffect, useId, useRef, useState, type ReactElement } from 'react'

import { CONTROL, narrowing } from '../../shell/controls'

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
      <Button
        variant="secondary"
        iconRight="expand_more"
        style={narrowing(chosen.length > 0)}
        aria-expanded={open}
        aria-haspopup="true"
        aria-controls={`${id}-list`}
        onClick={() => setOpen(!open)}
      >
        {label}
        {chosen.length > 0 ? <span className="filter__count">{chosen.length}</span> : null}
      </Button>

      {open ? (
        <div className="filter__popover" id={`${id}-list`}>
          {/* The entries already carried the design system's shape — `label`,
              `checked`, `onClick` — so the popover is now its Menu, and
              "Clear" is an entry in it rather than a button parked underneath. */}
          <Menu
            width={232}
            items={[
              ...items,
              ...(chosen.length > 0
                ? ([
                    { divider: true },
                    {
                      label: `Clear ${label.toLowerCase()}`,
                      icon: 'backspace',
                      onClick: () => onChange(undefined),
                    },
                  ] as MenuEntry[])
                : []),
            ]}
          />
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
      <Select
        // Content-sized, with a ceiling. A toolbar control that stretches to
        // fill the row makes the row's width, not its own contents, decide how
        // important it looks.
        width="auto"
        containerStyle={{ maxWidth: 200 }}
        id={id}
        style={{ height: CONTROL, ...narrowing(value !== undefined && value !== '') }}
        value={value ?? ''}
        onChange={(event) => onChange(event.target.value === '' ? undefined : event.target.value)}
      >
        {options.map((option) => (
          <option key={option.value} value={option.value}>
            {option.label}
          </option>
        ))}
      </Select>
    </>
  )
}
