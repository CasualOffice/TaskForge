/**
 * How a refusal, and how an absence, reach the screen.
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
 *
 * # Why the gap notice became one grey line
 *
 * It used to be a boxed, tinted panel carrying a tracker id (`C-010`), an HTTP
 * method and a path. Three of them stacked in the task drawer, identical, and
 * the reader learned nothing: nobody outside this repository knows what C-010 is
 * or why `POST /api/v1/tasks/{id}/attachments` matters to them.
 * `design/VISUAL-IDENTITY.md` §9 asks for concise, factual, operational voice
 * and no jargon; three developer-facing boxes is the opposite, and it made the
 * *product* look unfinished rather than the feature.
 *
 * So there is one pattern, used everywhere: **a single quiet line, in the
 * reader's language, saying what is not there yet.** No box, no colour, no
 * tracker id, no endpoint. The engineering detail did not disappear — it lives
 * in `docs/14-EXECUTION-TRACKER.md`, which is where a reader who can act on it
 * looks, and in the module comment of the file that is waiting for it.
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
 * A capability the server does not serve yet — one line, no emphasis.
 *
 * Distinct from an error on purpose: an error is something that went wrong, and
 * a gap is something that was never built. Rendering the second as the first
 * teaches users to ignore error styling. Rendering it as nothing at all makes
 * the product look like it silently lost a feature. Rendering it *loudly*, which
 * is what this used to do, makes a reader feel they have hit a wall in every
 * section they open.
 *
 * `what` is a complete sentence a user can understand without this repository.
 */
export function GapNotice({ what, children }: { what: string; children?: ReactNode }): ReactElement {
  return (
    <p className="gapline">
      {what}
      {children === undefined ? null : <> {children}</>}
    </p>
  )
}
