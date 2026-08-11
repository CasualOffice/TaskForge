/**
 * The workspace: which one you are in, which others you can reach, and how to
 * start another.
 *
 * # Why this is one control and not three
 *
 * It was a bare name, a native `<select>` that only appeared once a second
 * workspace existed, and a separate button to make one. Three controls for one
 * subject, each looking like something else: the name looked like a label, the
 * select looked like a filter, and a plus beside a workspace name could plainly
 * mean "add anything". A person with a single workspace saw a word they could
 * not click and no evidence there was anything to switch *to*.
 *
 * So it is one button that shows where you are, and a menu that holds
 * everywhere you could be and the two things you can do about it. That is the
 * shape every product this one is measured against uses, and the reason is not
 * fashion: the workspace is a *place*, and a place is chosen from a list of
 * places, not set like a preference.
 *
 * # Why the mark is an initial and not an image
 *
 * A workspace has no avatar in this schema, and inventing an upload for one is
 * a feature. An initial in a tinted square is enough to tell two workspaces
 * apart at a glance in a list, which is the whole job — and the tint is derived
 * from the id, so it is stable without being stored.
 *
 * # Why switching is not a `<select>` any more
 *
 * A native select was the right call when this was a form control, and its
 * accessibility and touch behaviour came free. As a menu the same guarantees
 * have to be earned: the trigger carries `aria-haspopup` and `aria-expanded`
 * from `Popover`, each row is a real `<button>` with `aria-current` on the one
 * you are in, and `Popover` already handles Escape, click-away and focus
 * return. What is *not* free is type-ahead, and a list of workspaces is short
 * enough that its absence costs nothing.
 */
import { useState, type ReactElement } from 'react'
import { Link } from '@tanstack/react-router'

import { NewWorkspace } from './NewWorkspace'
import { Popover } from './Popover'
import { useSession } from './session'

/**
 * A stable tint from an id.
 *
 * Six hues, chosen by summing the id's characters. Not random: the same
 * workspace has to be the same colour on every load, or the mark stops being a
 * way to recognise it and becomes decoration.
 */
function tintOf(id: string): string {
  let sum = 0
  for (const character of id) sum += character.codePointAt(0) ?? 0
  return `var(--tf-mark-${(sum % 6) + 1})`
}

function Mark({ name, id }: { name: string; id: string }): ReactElement {
  return (
    <span className="wsmark" style={{ background: tintOf(id) }} aria-hidden="true">
      {[...name][0]?.toUpperCase() ?? '?'}
    </span>
  )
}

export function WorkspaceSwitcher(): ReactElement | null {
  const { workspaces, workspace, chooseWorkspace } = useSession()

  if (workspace === undefined) return null

  return (
    <Popover
      triggerClass="wsbtn"
      ariaLabel={`Workspace: ${workspace.name}. Switch or create one.`}
      label={
        <>
          <Mark name={workspace.name} id={workspace.id} />
          <span className="wsbtn__name">{workspace.name}</span>
          <span className="wsbtn__chevron material-symbols-outlined" aria-hidden="true">
            expand_more
          </span>
        </>
      }
    >
      {(close) => (
        <WorkspaceMenu
          close={close}
          current={workspace.id}
          workspaces={workspaces}
          onChoose={chooseWorkspace}
        />
      )}
    </Popover>
  )
}

function WorkspaceMenu({
  close,
  current,
  workspaces,
  onChoose,
}: {
  close: () => void
  current: string
  workspaces: readonly { id: string; name: string }[]
  onChoose: (id: string) => void
}): ReactElement {
  // The form replaces the list in place rather than opening a second surface
  // over the first. A popover on a popover is two dismissal targets and two
  // focus traps, and the list is not information you need while naming a new
  // workspace.
  const [creating, setCreating] = useState(false)

  if (creating) {
    return (
      <div className="wsmenu">
        <div className="wsmenu__head">
          <button type="button" className="wsmenu__back" onClick={() => setCreating(false)}>
            <span className="material-symbols-outlined" aria-hidden="true">
              arrow_back
            </span>
            Workspaces
          </button>
        </div>
        <h2 className="wsmenu__heading">Start a workspace</h2>
        <NewWorkspace onDone={close} />
      </div>
    )
  }

  return (
    <div className="wsmenu">
      <h2 className="wsmenu__heading" id="wsmenu-title">
        Workspaces
      </h2>
      <ul className="pop__list" aria-labelledby="wsmenu-title">
        {workspaces.map((candidate) => (
          <li key={candidate.id}>
            <button
              type="button"
              className="pop__item wsmenu__row"
              aria-current={candidate.id === current ? 'true' : undefined}
              onClick={() => {
                onChoose(candidate.id)
                close()
              }}
            >
              {/* `pop__item[aria-current]` already draws the tick — the shared
                  menu item has carried one since it was written, and adding a
                  second here put two check marks on the row you are already in.
                  The attribute is what marks it; the indicator belongs to the
                  component that owns the row. */}
              <Mark name={candidate.name} id={candidate.id} />
              <span className="wsmenu__name">{candidate.name}</span>
            </button>
          </li>
        ))}
      </ul>

      <div className="wsmenu__foot">
        <button type="button" className="pop__item" onClick={() => setCreating(true)}>
          <span className="material-symbols-outlined" aria-hidden="true">
            add
          </span>
          Create workspace
        </button>
        <Link className="pop__item" to="/settings/workspace" onClick={close}>
          <span className="material-symbols-outlined" aria-hidden="true">
            settings
          </span>
          Workspace settings
        </Link>
      </div>
    </div>
  )
}
