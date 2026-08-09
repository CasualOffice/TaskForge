/** Workspace primary-colour control, shown only with `workspace.manage`. */
import { useEffect, useState, type CSSProperties, type ReactElement } from 'react'
import { useMutation, useQueryClient } from '@tanstack/react-query'

import { keys } from '../api/keys'
import { PERMISSIONS } from '../api/permissions'
import { updateWorkspaceAppearance } from '../api/workspaces'
import { useAuthority } from './permissions'
import { Popover } from './Popover'
import { applyWorkspacePrimary } from './appearance'
import { ErrorNotice } from './notice'
import { useSession } from './session'

const PRESETS = ['#2563EB', '#1D4ED8', '#0F766E', '#6D28D9', '#BE123C'] as const

export function AppearanceControl(): ReactElement | null {
  const { workspace } = useSession()
  const { can } = useAuthority()
  const client = useQueryClient()
  const [colour, setColour] = useState(workspace?.appearance?.primary_color ?? PRESETS[0])

  useEffect(() => {
    setColour(workspace?.appearance?.primary_color ?? PRESETS[0])
  }, [workspace?.appearance?.primary_color, workspace?.id])

  const update = useMutation({
    mutationFn: async (primary: string) => {
      if (workspace === undefined) throw new Error('workspace missing')
      return updateWorkspaceAppearance(workspace, primary)
    },
    onSuccess: async (next) => {
      applyWorkspacePrimary(next.appearance.primary_color)
      await client.invalidateQueries({ queryKey: keys.workspaces() })
    },
  })

  if (workspace === undefined || !can(PERMISSIONS.workspaceManage)) return null

  return (
    <Popover
      label={
        <>
          <span className="top-action__swatch" aria-hidden="true" />
          <span className="top-action__label">Appearance</span>
        </>
      }
      ariaLabel="Workspace appearance"
      align="end"
      triggerClass="button button--quiet top-action"
    >
      {(close) => (
        <form
          className="appearance-panel"
          onSubmit={(event) => {
            event.preventDefault()
            update.mutate(colour.toUpperCase(), { onSuccess: close })
          }}
        >
          <div>
            <p className="appearance-panel__title">Workspace colour</p>
            <p className="appearance-panel__hint">Used for actions and selected items.</p>
          </div>
          <div className="appearance-panel__swatches" aria-label="Colour presets">
            {PRESETS.map((preset) => (
              <button
                key={preset}
                type="button"
                className="appearance-panel__swatch"
                style={{ '--swatch': preset } as CSSProperties}
                aria-label={`Use ${preset}`}
                aria-pressed={colour.toUpperCase() === preset}
                onClick={() => setColour(preset)}
              />
            ))}
          </div>
          <label className="appearance-panel__picker">
            <span>Custom colour</span>
            <input
              type="color"
              value={colour}
              onChange={(event) => setColour(event.target.value)}
            />
          </label>
          {update.isError ? <ErrorNotice error={update.error} /> : null}
          <div className="appearance-panel__actions">
            <button type="button" className="button button--quiet" onClick={close}>
              Cancel
            </button>
            <button type="submit" className="button button--primary" disabled={update.isPending}>
              {update.isPending ? 'Saving…' : 'Save'}
            </button>
          </div>
        </form>
      )}
    </Popover>
  )
}
