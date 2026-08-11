/**
 * Which of the task surface's declared sections have nothing behind them yet.
 *
 * # Why this reads the registry rather than a literal list
 *
 * `docs/34` and C-017 require the core's own panels to be declared in the
 * extension point registry so the seam is exercised before a plugin depends on
 * it. Skipping a declared contribution would make the registry decorative — the
 * surface would look identical whether the contribution existed or not.
 *
 * # Why the *layout* is no longer driven by it
 *
 * `design/LAYOUT-AND-INTERACTION-GUIDELINES.md` §4 now specifies columns rather
 * than a stack: identity and narrative on the left, metadata on the right. A
 * registry that returns an ordered list cannot express which column a section
 * belongs in, and pretending it can would have produced a single column again —
 * which is the shape the specification just rejected. So the registry decides
 * *what exists*, this file decides *what is missing*, and the surface decides
 * where a section sits. When the registry gains an HTTP surface it can carry a
 * placement hint; until then that mapping is honest code rather than an invented
 * field.
 *
 * # One sentence, not one box per section
 *
 * Three identical tinted panels reading "Registered in the extension point
 * registry; nothing serves it yet" told a reader nothing they could act on and
 * made the product look broken. The names come from the registry; the sentence
 * is in the reader's language.
 */
import { contributions } from '../extensions/coreContributions'

/**
 * Panels the client renders itself. Everything else declared is still to come.
 *
 * `attachments` was deliberately absent from this list for a long time, and the
 * reason is worth keeping now that it is gone: the pipeline existed — presign,
 * commit, scan, download — but the presigned upload URL pointed at an object
 * origin the browser refused to `PUT` to, because that router served no
 * `OPTIONS` and a cross-origin upload is preflighted. A client could get a URL
 * and had nowhere to send the bytes, so an upload control would have been a
 * button that always failed.
 *
 * The preflight is served, so the control is real and the panel is listed.
 */
const IMPLEMENTED: ReadonlySet<string> = new Set([
  'details',
  'comments',
  'relations',
  'activity',
  'subtasks',
  'attachments',
])

/** The declared-but-unserved section titles, lowercased for a sentence. */
export function unbuiltSections(): readonly string[] {
  return contributions('ui.task.panel')
    .filter((entry) => !IMPLEMENTED.has(entry.slug))
    .map((entry) => entry.title.toLowerCase())
}

/**
 * "Not available yet: attachments." — or `undefined` when everything declared is
 * served, at which point the line disappears rather than becoming a boast.
 *
 * # Why the verb went away
 *
 * It read "Attachments is not available yet" once only one panel was left. The
 * agreement was computed from *how many sections* there were, and every section
 * name is itself a plural word — so one section produced a singular verb against
 * a plural noun. Naming them after a colon has no verb to get wrong, however
 * many there are.
 */
export function unbuiltSentence(): string | undefined {
  const names = unbuiltSections()
  const last = names[names.length - 1]
  if (last === undefined) return undefined
  const list = names.length === 1 ? last : `${names.slice(0, -1).join(', ')} and ${last}`
  return `Not available yet: ${list}.`
}
