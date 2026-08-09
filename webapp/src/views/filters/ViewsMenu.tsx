/**
 * The built-in views, as one control.
 *
 * # Why views come before a filter builder
 *
 * `docs/27` §Built-in views ships seven named filters, and they exist because
 * "a filter you cannot save is a filter you retype". Six controls in a toolbar
 * let someone *construct* "my overdue work"; a view lets them *ask* for it. The
 * seven are the questions people actually have, and they are already written
 * down — so they are the highest-value filtering surface in the product and the
 * cheapest, because they need no storage.
 *
 * # Saved views are not here, and the reason is an endpoint
 *
 * `docs/27` §Saved views specifies user-authored views with sharing, ownership
 * and a layout, and `docs/05` lists `GET/POST /api/v1/saved-views`. No route is
 * registered for it. Persisting them in this browser instead would produce views
 * that cannot be shared, do not follow the user to another machine, and silently
 * disappear when storage is cleared — three properties the specification
 * explicitly rejects. So the control offers what is real and says what is not.
 *
 * # A withheld view is shown, disabled, with the reason
 *
 * Two built-ins need server behaviour that currently answers `TF-SYS-0001` (see
 * `tasks/builtinViews.ts`). Hiding them would quietly redefine what TaskForge
 * ships as what it currently manages; running them would show the user a 500.
 */
import { useEffect, useRef, useState, type ReactElement } from 'react'

import type { AppSearch } from '../../router'
import { BUILTIN_VIEWS, activeView, type BuiltinView } from '../../tasks/builtinViews'

export function ViewsMenu({
  search,
  onApply,
}: {
  search: AppSearch
  onApply: (view: BuiltinView) => void
}): ReactElement {
  const [open, setOpen] = useState(false)
  const host = useRef<HTMLDivElement>(null)
  const current = activeView(search)

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

  const mine = BUILTIN_VIEWS.filter((view) => view.mine)
  const shared = BUILTIN_VIEWS.filter((view) => !view.mine)

  return (
    <div className="filter" ref={host}>
      <button
        type="button"
        className={`button filter__trigger${current === undefined ? '' : ' filter__trigger--on'}`}
        aria-expanded={open}
        aria-haspopup="true"
        onClick={() => setOpen(!open)}
      >
        {current === undefined ? 'Views' : current.name}
      </button>

      {open ? (
        <div className="filter__popover views__popover">
          <Group label="My Work" views={mine} current={current} onApply={onApply} close={() => setOpen(false)} />
          <Group label="Workspace" views={shared} current={current} onApply={onApply} close={() => setOpen(false)} />
          <p className="views__note">
            Saving your own view needs <code>/api/v1/saved-views</code>, which is specified in
            docs/05 and not served yet.
          </p>
        </div>
      ) : null}
    </div>
  )
}

function Group({
  label,
  views,
  current,
  onApply,
  close,
}: {
  label: string
  views: readonly BuiltinView[]
  current: BuiltinView | undefined
  onApply: (view: BuiltinView) => void
  close: () => void
}): ReactElement {
  return (
    <>
      <p className="views__group">{label}</p>
      <ul className="filter__options">
        {views.map((view) => (
          <li key={view.id}>
            <button
              type="button"
              className={`views__item${current?.id === view.id ? ' views__item--on' : ''}`}
              disabled={view.unavailable !== undefined}
              title={view.unavailable}
              onClick={() => {
                onApply(view)
                close()
              }}
            >
              <span>{view.name}</span>
              {view.unavailable === undefined ? null : (
                <span className="views__blocked">unavailable</span>
              )}
            </button>
          </li>
        ))}
      </ul>
    </>
  )
}
