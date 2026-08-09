/**
 * How a refusal reaches the screen.
 *
 * # The failure this module prevents
 *
 * A raw 500 body, a status line, or a stack trace rendered at a user. `docs/05`
 * gives every refusal a registry code so the client can say something the user
 * can act on; `api/problem.ts` turns the code into that sentence, and this is
 * the only component that puts one on screen. There is deliberately no prop for
 * "message" — a caller cannot pass its own text and bypass the table.
 *
 * The `request_id` is shown for every error, because `docs/05` promises the user
 * something to quote to support and the moment they need it is exactly this one.
 */
import type { ReactElement, ReactNode } from 'react'

import { asApiError } from '../api/problem'

export function ErrorNotice({ error }: { error: unknown }): ReactElement {
  const api = asApiError(error)
  return (
    <div className="notice notice--error" role="alert">
      <span>{api.sentence}</span>
      <span className="notice__meta">
        {api.code}
        {api.requestId === undefined ? '' : ` · request ${api.requestId}`}
      </span>
    </div>
  )
}

/**
 * A capability the server does not serve yet.
 *
 * Distinct from an error on purpose: an error is something that went wrong, and
 * a gap is something that was never built. Rendering the second as the first
 * teaches users to ignore error styling, and rendering it as nothing at all
 * makes the product look like it silently lost a feature.
 *
 * `tracker` names the row in `docs/14-EXECUTION-TRACKER.md` that closes it, so
 * the gap on screen and the gap in the record are the same gap.
 */
export function GapNotice({
  what,
  tracker,
  children,
}: {
  what: string
  tracker: string
  children?: ReactNode
}): ReactElement {
  return (
    <div className="notice notice--gap">
      <strong>{what}</strong>
      {children}
      <span className="notice__meta">tracked as {tracker}</span>
    </div>
  )
}
