/**
 * Starting a workspace.
 *
 * # The dead end this removes
 *
 * A person who signed in and belonged to nothing was shown "You are signed in
 * but belong to no workspace yet. Ask an owner for an invitation." — a screen
 * with no control on it. The API had `POST /api/v1/workspaces` the whole time
 * and the client never called it, so the product's first-run state was a
 * message telling you to go and find someone else.
 *
 * # Why the slug is offered rather than demanded
 *
 * A slug is a URL detail, and asking for one before the workspace exists is
 * asking someone to make a decision about a thing they have not made yet. So it
 * is derived from the name as it is typed — "Acme, Inc." becomes `acme-inc` —
 * and stays editable, because the derivation is a good guess and not a rule.
 * The moment it is edited by hand it stops following the name: a field that
 * silently overwrote what someone typed would be worse than one that never
 * offered.
 *
 * # Why it says what the slug is for
 *
 * `docs/32`: the slug is the tenant's stable public handle. It is worth one
 * line saying so, because "acme-inc" beside an unexplained label reads as a
 * required field of unknown consequence, and people either skip it or agonise.
 */
import { useState, type ReactElement } from 'react'
import { Button, Input } from '@schnsrw/design-system'
import { useMutation, useQueryClient } from '@tanstack/react-query'

import { createWorkspace, slugFrom } from '../api/workspaces'
import { ErrorNotice } from './notice'
import { useSession } from './session'

/** The server's own rule, checked here so the refusal arrives before the request. */
const SLUG_SHAPE = /^[a-z0-9][a-z0-9-]{0,63}$/

export function NewWorkspace({ onDone }: { onDone?: () => void }): ReactElement {
  const client = useQueryClient()
  const { chooseWorkspace } = useSession()

  const [name, setName] = useState('')
  // `undefined` means "still following the name". A separate flag rather than
  // comparing the two strings: typing a slug that happens to match its derived
  // form must not silently re-arm the derivation.
  const [slug, setSlug] = useState<string | undefined>(undefined)

  const derived = slugFrom(name)
  const effective = slug ?? derived

  const create = useMutation({
    mutationFn: () => createWorkspace({ name: name.trim(), slug: effective }),
    onSuccess: async (workspace) => {
      // The whole tenant prefix, because every cached answer was about a
      // workspace that is not this one — the same reason a switch invalidates.
      await client.invalidateQueries({ queryKey: ['ws'] })
      chooseWorkspace(workspace.id)
      onDone?.()
    },
  })

  const usable = name.trim() !== '' && SLUG_SHAPE.test(effective)

  return (
    <form
      className="newws"
      onSubmit={(event) => {
        event.preventDefault()
        if (usable) create.mutate()
      }}
    >
      <p className="field">
        <label className="field__label" htmlFor="ws-name">
          Workspace name
        </label>
        <Input
          full
          id="ws-name"
          value={name}
          maxLength={200}
          autoComplete="organization"
          onChange={(event) => setName(event.target.value)}
        />
      </p>

      <p className="field">
        <label className="field__label" htmlFor="ws-slug">
          Address
        </label>
        <Input
          full
          id="ws-slug"
          value={effective}
          maxLength={64}
          onChange={(event) => setSlug(event.target.value.toLowerCase())}
        />
        <span className="field__hint">
          {effective === '' ? (
            'Lower-case letters, digits and hyphens. Used in links to this workspace.'
          ) : SLUG_SHAPE.test(effective) ? (
            <>
              Used in links to this workspace. It cannot be changed here later.
            </>
          ) : (
            'Lower-case letters, digits and hyphens, starting with a letter or digit.'
          )}
        </span>
      </p>

      {create.error === null ? null : <ErrorNotice error={create.error} />}

      <Button variant="primary" type="submit" disabled={create.isPending || !usable}>
        {create.isPending ? 'Creating…' : 'Create workspace'}
      </Button>
    </form>
  )
}
