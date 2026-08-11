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
import { useState, type ReactElement } from 'react'
import { Button, Select } from '@schnsrw/design-system'

import { NewWorkspace } from './NewWorkspace'
import { useSession } from './session'

export function WorkspaceSwitcher(): ReactElement | null {
  const { workspaces, workspace, chooseWorkspace } = useSession()
  const [creating, setCreating] = useState(false)

  if (workspace === undefined) return null

  // The form, in place of the switcher, until it is done. A dialog would be the
  // obvious alternative and is the wrong one here: this is the outermost scope
  // of the whole application, and a modal over the workspace you are leaving
  // shows you the old tenant behind the form that replaces it.
  if (creating) {
    return (
      <div className="workspace__new">
        <NewWorkspace onDone={() => setCreating(false)} />
        <Button size="sm" onClick={() => setCreating(false)}>
          Cancel
        </Button>
      </div>
    )
  }

  const add = (
    /* Reachable whether or not there is a second workspace to switch to. The
       switcher only renders as a `<select>` when there are two, so hanging
       "New workspace" off the select alone would have hidden it from exactly
       the person most likely to want it: someone with one workspace. */
    <Button size="sm" variant="subtle" icon="add" onClick={() => setCreating(true)}>
      New workspace
    </Button>
  )

  if (workspaces.length === 1) {
    return (
      <div className="workspace__row">
        <span className="workspace__single">{workspace.name}</span>
        {add}
      </div>
    )
  }

  return (
    <div className="workspace__row">
      <label className="visually-hidden" htmlFor="workspace-switcher">
        Workspace
      </label>
      <Select
        width="auto"
        containerStyle={{ maxWidth: 220 }}
        id="workspace-switcher"
        value={workspace.id}
        onChange={(event) => chooseWorkspace(event.target.value)}
      >
        {workspaces.map((candidate) => (
          <option key={candidate.id} value={candidate.id}>
            {candidate.name}
          </option>
        ))}
      </Select>
      {add}
    </div>
  )
}
