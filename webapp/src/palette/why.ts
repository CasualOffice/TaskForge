/**
 * Why a task came back for what somebody typed.
 *
 * # The failure this module prevents
 *
 * A result list that looks random. The search projection (`docs/26`
 * §Weighting) indexes four bands — the key and title in `A`, tag names and the
 * assignee, reporter and milestone in `B`, the description in `C`, comment
 * bodies in `D` — but the palette row shows only the key and the title. So a
 * task matched on its description, or on the name of the person who raised it,
 * arrives looking like a task that matched on nothing.
 *
 * That is not hypothetical. Typing a colleague's name returned every task they
 * had ever raised, because their display name is in band `B` of each one, and
 * the eight rows on screen shared no visible word with the query. A search that
 * is working correctly and cannot say so is indistinguishable from a broken one.
 *
 * # Why this is inferred on the client rather than asked of the server
 *
 * PostgreSQL will say precisely, with `ts_headline`, and that is the better
 * answer — it knows which lexeme matched, this does not. It also costs a second
 * pass over the document for every row of every keystroke, and it is a change to
 * the wire type. This is the cheap half: the row already carries `title`,
 * `description`, `reporter_id` and `assignees`, and the palette already holds
 * the workspace's members, so the common cases can be named with no request at
 * all.
 *
 * The unavoidable consequence is that this can be **wrong about the reason**
 * while the match itself is right — a stemmed match (`restoring` for `restore`)
 * or a hit in a comment or tag body, neither of which is on the row. So the
 * fallback says where it did *not* match rather than claiming where it did, and
 * nothing here is ever load-bearing: it is a subtitle, not an assertion.
 */

/** Who the ids on a task row refer to. */
export type People = ReadonlyMap<string, string>

interface Matchable {
  readonly title: string
  readonly description: string | null
  readonly reporter_id: string
  readonly assignees?: readonly string[]
}

/**
 * A short note for a task row, or `undefined` when the title already explains
 * itself.
 *
 * Returning `undefined` for the obvious case is the point: annotating a row
 * whose title visibly contains the query is noise on every row, and noise on
 * every row is what makes a real note invisible.
 */
export function whyMatched(task: Matchable, term: string, people: People): string | undefined {
  const needle = term.trim().toLowerCase()
  if (needle === '') return undefined

  // The title is on screen. If the match is visible there, say nothing.
  if (task.title.toLowerCase().includes(needle)) return undefined

  // A person before a body of text: "reported by Ash" is a reason somebody can
  // act on, and it is the case that produced the confusing result.
  const named = (id: string | undefined): string | undefined => {
    if (id === undefined) return undefined
    const name = people.get(id)
    return name !== undefined && name.toLowerCase().includes(needle) ? name : undefined
  }

  const assignee = (task.assignees ?? []).map(named).find((name) => name !== undefined)
  if (assignee !== undefined) return `assigned to ${assignee}`

  const reporter = named(task.reporter_id)
  if (reporter !== undefined) return `reported by ${reporter}`

  if (task.description !== null && task.description.toLowerCase().includes(needle)) {
    return 'matches the description'
  }

  // Everything left: a comment, a tag, a milestone, or a stemmed form of the
  // word. Stated as the honest negative rather than guessed at — claiming "in a
  // comment" for a stemmed title match would be a confident wrong answer where
  // a vague right one costs nothing.
  return 'matches elsewhere in the task'
}
