/**
 * Choosing the tenant.
 *
 * # The failure this module prevents
 *
 * A workspace switch that leaves the previous tenant's rows on screen. Switching
 * is not a cosmetic change of label: it changes the `X-Workspace-Id` on every
 * subsequent request, so every cached answer under the old tenant is now about a
 * workspace the user is not looking at. `session.tsx` invalidates the whole
 * `['ws', …]` prefix for that reason, and this control's only job is to call it.
 *
 * A native `<select>`, not a custom menu. It is keyboard-operable, screen-reader
 * labelled, and touch-correct on every platform in `docs/18` for zero bytes —
 * and a bespoke listbox that reimplements all three is the classic place a
 * keyboard trap appears (docs/42 §Accessibility).
 */
import type { ReactElement } from 'react'

import { useSession } from './session'

export function WorkspaceSwitcher(): ReactElement | null {
  const { workspaces, workspace, chooseWorkspace } = useSession()

  if (workspace === undefined) return null
  if (workspaces.length === 1) {
    return <span className="workspace__single">{workspace.name}</span>
  }

  return (
    <>
      <label className="visually-hidden" htmlFor="workspace-switcher">
        Workspace
      </label>
      <select
        id="workspace-switcher"
        className="select workspace__select"
        value={workspace.id}
        onChange={(event) => chooseWorkspace(event.target.value)}
      >
        {workspaces.map((candidate) => (
          <option key={candidate.id} value={candidate.id}>
            {candidate.name}
          </option>
        ))}
      </select>
    </>
  )
}
