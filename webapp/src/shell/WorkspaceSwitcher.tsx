/**
 * Choosing the tenant, and starting one.
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
 *
 * # Why it lives in the header
 *
 * The workspace is the outermost scope: the rail, every route under it and
 * every request carry it. In the rail it read as one more item *inside* the
 * navigation, which inverts that — the container listed among the things it
 * contains. The header is the one band that spans the whole window, which is
 * the shape of what a workspace is.
 *
 * # Why creating one is a popover
 *
 * It is a two-field form reached from a control that is one word wide. Giving
 * it a route would be navigating away from the workspace you are in to make a
 * different one; putting it inline in the header would push the entire bar
 * open. A popover is anchored to the thing it is about and takes nothing away
 * while it is open.
 */
import type { ReactElement } from 'react'
import { Icon, Select } from '@schnsrw/design-system'

import { NewWorkspace } from './NewWorkspace'
import { Popover } from './Popover'
import { useSession } from './session'

export function WorkspaceSwitcher(): ReactElement | null {
  const { workspaces, workspace, chooseWorkspace } = useSession()

  if (workspace === undefined) return null

  return (
    <div className="workspace">
      {workspaces.length === 1 ? (
        <span className="workspace__single">{workspace.name}</span>
      ) : (
        <>
          <label className="visually-hidden" htmlFor="workspace-switcher">
            Workspace
          </label>
          <Select
            width="auto"
            containerStyle={{ maxWidth: 200 }}
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
        </>
      )}

      {/* Offered whether or not there is a second workspace to switch to. The
          switcher only becomes a `<select>` when there are two, so hanging this
          off the select would hide it from exactly the person most likely to
          want it: someone with one workspace. */}
      {/* An icon, not the words. "New workspace" spelled out in the header put
          a rarely-used action at the same weight as search — it is a plus beside
          the name it adds to, and its accessible name carries the sentence. */}
      <Popover
        label={<Icon name="add" size="sm" />}
        ariaLabel="New workspace"
        title="New workspace"
        triggerVariant="subtle"
      >
        {(close) => (
          <div className="workspace__form">
            <h2 className="workspace__heading">Start a workspace</h2>
            <NewWorkspace onDone={close} />
          </div>
        )}
      </Popover>
    </div>
  )
}
