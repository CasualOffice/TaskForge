/**
 * The pieces every settings screen is built from.
 *
 * # Why a shared `useWrite` and not `useMutation` at each call site
 *
 * Seven screens performing thirty writes need the same four things every time:
 * invalidate what the write changed, announce what happened, keep the refusal on
 * screen where the control is, and never leave a button spinning after a
 * failure. Written per call site, the third one gets forgotten — the toast
 * disappears after five seconds and the user is left looking at a form that
 * silently did nothing.
 *
 * So a refusal is held *beside the control* until the next attempt, and the
 * announcement is the same sentence rather than a second wording of it.
 *
 * # Why the section, not a card component
 *
 * Layout here is structural: a heading, a description, and the controls. Codex
 * owns the visual design, and a component that baked in borders and shadows
 * would be a second design system to unpick. These carry class names and no
 * decoration.
 */
import { useCallback, useState, type FormEvent, type ReactElement, type ReactNode } from 'react'
import { useMutation, useQueryClient, type QueryKey } from '@tanstack/react-query'

import { asApiError } from '../api/problem'
import { SkeletonRows } from '../shell/Skeleton'
import { useAnnounce } from '../shell/announce'
import { ErrorNotice } from '../shell/notice'

/**
 * The placeholder every settings list shows while it loads.
 *
 * The height is fixed here rather than at each call site so the rows do not jump
 * when the data lands — which is the entire point of a skeleton, and the part a
 * caller passing its own number gets subtly wrong.
 */
export function Loading({ rows = 3, label }: { rows?: number; label: string }): ReactElement {
  return <SkeletonRows rows={rows} height={44} label={label} />
}

/**
 * The page's own header (`docs/49` §4).
 *
 * Every settings route used to inherit one `<h1>` — the word "Settings", which
 * lives in the navigation — so no page ever named itself. A reader who followed
 * "your admin can change that under Roles" arrived somewhere whose only heading
 * said "Settings", and had to read the list on the left to work out where they
 * were.
 *
 * The primary action sits with the heading rather than below the list it adds
 * to, which is where every product this one is measured against puts it.
 */
export function PageHead({
  title,
  description,
  children,
  actions,
}: {
  title: string
  description?: string
  children: ReactNode
  actions?: ReactNode
}): ReactElement {
  return (
    <section className="settings__section">
      <header className="settings__section-head">
        <div>
          <h1 className="settings__page-title">{title}</h1>
          {description === undefined ? null : (
            <p className="settings__section-desc">{description}</p>
          )}
        </div>
        {actions === undefined ? null : <div className="settings__section-actions">{actions}</div>}
      </header>
      {children}
    </section>
  )
}

export function Section({
  title,
  description,
  children,
  actions,
}: {
  title: string
  description?: string
  children: ReactNode
  actions?: ReactNode
}): ReactElement {
  return (
    <section className="settings__section">
      <header className="settings__section-head">
        <div>
          <h2 className="settings__section-title">{title}</h2>
          {description === undefined ? null : (
            <p className="settings__section-desc">{description}</p>
          )}
        </div>
        {actions === undefined ? null : <div className="settings__section-actions">{actions}</div>}
      </header>
      {children}
    </section>
  )
}

/**
 * A write, with its refusal, its announcement and its invalidation in one place.
 *
 * `invalidates` is a list of key *prefixes*, matching TanStack's own matching:
 * a grant changes the grant list, the member list's badges and the actor's own
 * effective permissions, and naming all three here is what stops a screen
 * showing authority that no longer exists.
 */
export function useWrite<TArgs, TResult>(options: {
  run: (args: TArgs) => Promise<TResult>
  /** The past-tense sentence, from the arguments and the result. */
  announce: (result: TResult, args: TArgs) => string
  invalidates?: readonly QueryKey[]
  onDone?: (result: TResult, args: TArgs) => void
}): {
  readonly submit: (args: TArgs) => void
  readonly pending: boolean
  readonly error: unknown
  readonly clearError: () => void
} {
  const client = useQueryClient()
  const say = useAnnounce()
  const [error, setError] = useState<unknown>(undefined)

  const mutation = useMutation({
    mutationFn: options.run,
    onMutate: () => {
      // Cleared on attempt, not on success: a refusal that vanished the moment
      // the user touched the form would be gone before they read it.
      setError(undefined)
    },
    onSuccess: (result, args) => {
      for (const key of options.invalidates ?? []) {
        void client.invalidateQueries({ queryKey: key })
      }
      say(options.announce(result, args))
      options.onDone?.(result, args)
    },
    onError: (cause) => {
      setError(cause)
      // Announced as well as shown: the notice is beside the control, and a
      // screen-reader user may be nowhere near it.
      say(asApiError(cause).sentence, 'error')
    },
  })

  return {
    submit: useCallback((args: TArgs) => mutation.mutate(args), [mutation]),
    pending: mutation.isPending,
    error,
    clearError: useCallback(() => setError(undefined), []),
  }
}

/** The refusal, where the control is. Renders nothing when there is none. */
export function WriteError({ error }: { error: unknown }): ReactElement | null {
  if (error === undefined || error === null) return null
  return <ErrorNotice error={error} />
}

/**
 * A form that submits without reloading the page.
 *
 * A `<form>` rather than a button with an `onClick`, because Enter inside a text
 * field is how people submit one-field forms and a div cannot hear it.
 */
export function Form({
  onSubmit,
  children,
  className,
}: {
  onSubmit: () => void
  children: ReactNode
  className?: string
}): ReactElement {
  const handle = (event: FormEvent): void => {
    event.preventDefault()
    onSubmit()
  }
  return (
    <form className={className ?? 'settings__form'} onSubmit={handle}>
      {children}
    </form>
  )
}

/** A labelled control. The label is a real `<label>`, so clicking it focuses. */
export function Field({
  label,
  hint,
  children,
  id,
}: {
  label: string
  hint?: string
  children: ReactNode
  id: string
}): ReactElement {
  return (
    <p className="field">
      <label className="field__label" htmlFor={id}>
        {label}
      </label>
      {children}
      {hint === undefined ? null : <span className="field__hint">{hint}</span>}
    </p>
  )
}

/**
 * What a screen shows when the caller may not use it.
 *
 * Not a blank page and not a 403 after the first click: `docs/42` §Permissions
 * in the UI says affordances follow the resolved set, and a person who cannot
 * administer a workspace should be told that rather than shown controls that
 * refuse. It names the permission, because "ask an admin" without saying for
 * what is a message nobody can act on.
 */
export function NeedsPermission({ permission }: { permission: string }): ReactElement {
  return (
    <p className="empty">
      You do not have <code>{permission}</code> in this workspace. Someone with it can grant it to
      you under Roles.
    </p>
  )
}
