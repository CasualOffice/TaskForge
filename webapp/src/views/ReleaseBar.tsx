/**
 * Cut a release from what is selected (`docs/45` §The two clocks).
 *
 * # Why a bar and not a dialog
 *
 * The selection *is* the input. A dialog would cover the columns the reader
 * just picked from, and the first thing anyone does before naming a release is
 * look again at what is in it. So the bar sits under the board, names the
 * count, and leaves the board visible while it is filled in.
 *
 * # Why it says what will happen before it is pressed
 *
 * Cutting a release moves every selected task's environment. That is a real
 * change to eleven rows from one press, and a control that large has to state
 * its effect in the sentence next to it rather than in a toast afterwards.
 *
 * # Why a failure clears nothing
 *
 * The server refuses the batch whole — nothing moved — so the selection is
 * still exactly what the reader meant. Clearing it on failure would make them
 * rebuild it to try again with a different name.
 */
import { useState, type ReactElement } from 'react'
import { Button, Input, Select } from '@schnsrw/design-system'
import { useMutation, useQueryClient } from '@tanstack/react-query'

import { keys } from '../api/keys'
import type { Environment } from '../api/environments'
import { cutRelease } from '../api/releases'
import { CONTROL } from '../shell/controls'
import { ErrorNotice } from '../shell/notice'
import { useAnnounce } from '../shell/announce'

export function ReleaseBar({
  workspaceId,
  projectId,
  environments,
  selected,
  onCut,
  onClear,
}: {
  workspaceId: string
  projectId: string
  environments: readonly Environment[]
  selected: readonly string[]
  /** Called once the batch is recorded, so the board can refetch. */
  onCut: () => void
  onClear: () => void
}): ReactElement | null {
  const announce = useAnnounce()
  const client = useQueryClient()
  const [name, setName] = useState('')
  const [target, setTarget] = useState('')

  const cut = useMutation({
    mutationFn: () =>
      cutRelease(workspaceId, projectId, {
        name: name.trim(),
        environmentId: target,
        taskIds: selected,
      }),
    onSuccess: (result) => {
      announce(
        `${result.release.name} recorded — ${result.task_ids.length} ${
          result.task_ids.length === 1 ? 'task' : 'tasks'
        } moved.`,
      )
      setName('')
      setTarget('')
      onClear()
      onCut()
      void client.invalidateQueries({ queryKey: keys.releases(workspaceId, projectId) })
      void client.invalidateQueries({ queryKey: keys.taskLists(workspaceId) })
    },
  })

  if (selected.length === 0) return null

  const chosen = environments.find((environment) => environment.id === target)
  const ready = name.trim() !== '' && target !== ''

  return (
    <section className="relbar" aria-label="Cut a release">
      <div className="relbar__row">
        <strong className="relbar__count">{selected.length} selected</strong>
        <Button variant="subtle" size="sm" onClick={onClear}>
          Clear
        </Button>

        <label className="visually-hidden" htmlFor="release-target">
          Environment
        </label>
        <Select
          width="auto"
          containerStyle={{ maxWidth: 200 }}
          style={{ height: CONTROL }}
          id="release-target"
          value={target}
          onChange={(event) => setTarget(event.target.value)}
        >
          <option value="">Release to…</option>
          {environments.map((environment) => (
            <option key={environment.id} value={environment.id}>
              {environment.name}
            </option>
          ))}
        </Select>

        <label className="visually-hidden" htmlFor="release-name">
          Release name
        </label>
        <Input
          style={{ width: 200 }}
          id="release-name"
          value={name}
          placeholder="Name it — 2.4.0"
          onChange={(event) => setName(event.target.value)}
        />

        <Button variant="primary" disabled={!ready || cut.isPending} onClick={() => cut.mutate()}>
          {cut.isPending ? 'Recording…' : 'Cut release'}
        </Button>
      </div>

      {/* The sentence, not a toast afterwards: this press moves every selected
          task, and how many is the part worth reading twice. */}
      <p className="relbar__what">
        {chosen === undefined
          ? 'Choose where this went. Every selected task moves there.'
          : selected.length === 1
            ? `Records that one task went to ${chosen.name}, and moves it there.`
            : `Records that these ${selected.length} tasks went to ${chosen.name} together, and moves each one there.`}
      </p>

      {cut.error ? (
        <>
          <ErrorNotice error={cut.error} />
          {/* Said plainly, because "it failed" leaves a reader wondering
              whether some of it went through. None of it did. */}
          <p className="relbar__what">Nothing moved. The selection is unchanged.</p>
        </>
      ) : null}
    </section>
  )
}
