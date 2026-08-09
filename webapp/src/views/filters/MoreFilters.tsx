/**
 * The rest of the closed field set.
 *
 * # Why these are behind a control rather than in the toolbar
 *
 * `design/LAYOUT-AND-INTERACTION-GUIDELINES.md` §5 caps a toolbar at two rows
 * and says to move secondary actions into menus. The grammar has nineteen
 * filterable fields (`docs/27` §Fields and their operators); five of them answer
 * most questions and the other fourteen answer specific ones. Putting all of
 * them in the bar would push it to four rows and make the common five harder to
 * find — narrowing the interface in order to widen the filter.
 *
 * # Every control here is one row of the operator table
 *
 * `state` is `in` / `not_in`; `parent` is `is_empty` / `is_not_empty`; `archived`
 * is `eq` on a boolean the server defaults to false; `title` is `contains` and
 * deliberately not `q`, which is the full-text field and matches the description
 * too. Nothing here invents an operator a field does not permit — that is a
 * `400` by construction, and the client should not be able to compose one.
 *
 * # What is declared missing rather than omitted
 *
 * `tag`, `milestone` and `environment` are filterable in the grammar and have no
 * endpoint that lists their values, so there is no picker to build. `is_blocked`
 * is filterable and currently answers `TF-SYS-0001`. Both facts are on screen
 * rather than in a backlog, because a field silently absent from a filter panel
 * reads as a field the product does not have.
 */
import { useEffect, useRef, useState, type ReactElement } from 'react'

import { TASK_STATES } from '../../api/tasks'
import type { AppSearch } from '../../router'
import { stateLabel } from '../../tasks/present'

const RELATIVE_PRESETS: readonly { value: string; label: string }[] = [
  { value: '', label: 'Any time' },
  { value: '>-1d', label: 'In the last day' },
  { value: '>-7d', label: 'In the last week' },
  { value: '>-30d', label: 'In the last month' },
  { value: '<-30d', label: 'More than a month ago' },
]

export function MoreFilters({
  search,
  onChange,
}: {
  search: AppSearch
  onChange: (next: Partial<AppSearch>) => void
}): ReactElement {
  const [open, setOpen] = useState(false)
  const host = useRef<HTMLDivElement>(null)

  useEffect(() => {
    if (!open) return
    function onDocument(event: MouseEvent): void {
      if (!host.current?.contains(event.target as Node)) setOpen(false)
    }
    function onKey(event: KeyboardEvent): void {
      if (event.key === 'Escape') setOpen(false)
    }
    document.addEventListener('mousedown', onDocument)
    document.addEventListener('keydown', onKey)
    return () => {
      document.removeEventListener('mousedown', onDocument)
      document.removeEventListener('keydown', onKey)
    }
  }, [open])

  const count = [
    search.state,
    search.title,
    search.created,
    search.updated,
    search.parent,
    search.archived,
  ].filter((value) => value !== undefined && value !== '').length

  const states = search.state ?? ''
  const negated = states.startsWith('!')
  const chosenStates = states === '' ? [] : (negated ? states.slice(1) : states).split(',')

  function toggleState(state: string): void {
    const next = chosenStates.includes(state)
      ? chosenStates.filter((entry) => entry !== state)
      : [...chosenStates, state]
    onChange({
      state: next.length === 0 ? undefined : `${negated ? '!' : ''}${next.join(',')}`,
    })
  }

  return (
    <div className="filter" ref={host}>
      <button
        type="button"
        className={`button filter__trigger${count > 0 ? ' filter__trigger--on' : ''}`}
        aria-expanded={open}
        aria-haspopup="true"
        onClick={() => setOpen(!open)}
      >
        More
        {count > 0 ? <span className="filter__count">{count}</span> : null}
      </button>

      {open ? (
        <div className="filter__popover more__popover">
          <fieldset className="more__field">
            <legend className="more__legend">State</legend>
            <ul className="filter__options">
              {TASK_STATES.map((state) => (
                <li key={state}>
                  <label className="filter__option">
                    <input
                      type="checkbox"
                      checked={chosenStates.includes(state)}
                      onChange={() => toggleState(state)}
                    />
                    <span>{stateLabel(state)}</span>
                  </label>
                </li>
              ))}
            </ul>
            {chosenStates.length === 0 ? null : (
              <label className="filter__option">
                <input
                  type="checkbox"
                  checked={negated}
                  onChange={() =>
                    // `field=!a,b` is the grammar's `not_in`. Exposed as a switch
                    // rather than a separate control because it is the same
                    // selection asked the other way round.
                    onChange({ state: `${negated ? '' : '!'}${chosenStates.join(',')}` })
                  }
                />
                <span>Exclude these instead</span>
              </label>
            )}
          </fieldset>

          <Labelled label="Title contains">
            <input
              className="input"
              type="search"
              value={search.title ?? ''}
              placeholder="substring…"
              onChange={(event) =>
                onChange({ title: event.target.value === '' ? undefined : event.target.value })
              }
            />
          </Labelled>

          <Labelled label="Created">
            <Preset value={search.created} onChange={(next) => onChange({ created: next })} />
          </Labelled>

          <Labelled label="Updated">
            <Preset value={search.updated} onChange={(next) => onChange({ updated: next })} />
          </Labelled>

          <Labelled label="Nesting">
            <select
              className="select"
              value={search.parent ?? '__any'}
              onChange={(event) => {
                const chosen = event.target.value
                // `parent=` is `is_empty` — a task with no parent, i.e. top
                // level. The grammar has no `is_not_empty` spelling in the URL
                // form, so "subtasks only" is not offered rather than faked.
                onChange({ parent: chosen === '__any' ? undefined : '' })
              }}
            >
              <option value="__any">Everything</option>
              <option value="">Top-level tasks only</option>
            </select>
          </Labelled>

          <label className="filter__option">
            <input
              type="checkbox"
              checked={search.archived === 'true'}
              onChange={(event) => onChange({ archived: event.target.checked ? 'true' : undefined })}
            />
            <span>Include archived</span>
          </label>

          <p className="views__note">
            <code>tag</code>, <code>milestone</code> and <code>environment</code> are filterable
            and have no endpoint that lists their values. <code>is_blocked</code> answers
            TF-SYS-0001.
          </p>
        </div>
      ) : null}
    </div>
  )
}

function Labelled({ label, children }: { label: string; children: ReactElement }): ReactElement {
  return (
    <label className="more__field">
      <span className="more__legend">{label}</span>
      {children}
    </label>
  )
}

function Preset({
  value,
  onChange,
}: {
  value: string | undefined
  onChange: (next: string | undefined) => void
}): ReactElement {
  return (
    <select
      className="select"
      value={value ?? ''}
      onChange={(event) => onChange(event.target.value === '' ? undefined : event.target.value)}
    >
      {RELATIVE_PRESETS.map((preset) => (
        <option key={preset.value} value={preset.value}>
          {preset.label}
        </option>
      ))}
    </select>
  )
}
