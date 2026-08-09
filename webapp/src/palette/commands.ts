/**
 * What the palette can do, as data.
 *
 * # The failure this module prevents
 *
 * Navigation that grows an entry per feature. docs/42 §Command palette: "It is
 * why permanent navigation can stay at seven items — new capability adds a
 * command, not a nav entry." That only works if adding a command is cheaper than
 * adding a button, which means commands have to be *data* somewhere a
 * contributor can append to without touching the overlay's keyboard handling.
 *
 * `docs/34` extends the same idea to plugins: a plugin needs somewhere to put an
 * action that is not a new button on an already-crowded toolbar. The registry is
 * that somewhere, and the shape below is deliberately serializable — an id, a
 * title, a group, and a `run` — so a contributed command differs from a built-in
 * one only in who supplied it.
 */

export interface Command {
  readonly id: string
  readonly title: string
  /** Groups the list under a heading. Kept short: five groups is a menu, twenty is a maze. */
  readonly group: 'Go' | 'Task' | 'View'
  /** Extra words the search matches on but does not display — "cmd+k", "kanban". */
  readonly keywords?: string
  /** Shown right-aligned, e.g. a shortcut. Never load-bearing. */
  readonly hint?: string
  readonly run: () => void
}

/**
 * Rank commands against a typed query.
 *
 * Subsequence matching, not fuzzy scoring: "bd" finds "Board" because every
 * letter appears in order. It is a dozen lines, has no dependency, and a user
 * cannot tell it from a fuzzy matcher at this list size — where they *can* tell
 * is the 8 KiB a matching library would cost against an 86 KiB product budget.
 */
export function rank(commands: readonly Command[], term: string): readonly Command[] {
  const needle = term.trim().toLowerCase()
  if (needle === '') return commands

  const scored: { command: Command; score: number }[] = []
  for (const command of commands) {
    const haystack = `${command.title} ${command.keywords ?? ''}`.toLowerCase()
    const score = subsequenceScore(haystack, needle)
    if (score !== undefined) scored.push({ command, score })
  }
  // A stable sort, so equally-scoring commands keep their declared order rather
  // than reshuffling as the user types — a list that reorders under the cursor
  // is a list you cannot select from by muscle memory.
  return scored.sort((a, b) => a.score - b.score).map((entry) => entry.command)
}

/** Lower is better: the span the match occupies, plus where it starts. */
function subsequenceScore(haystack: string, needle: string): number | undefined {
  let index = 0
  let first = -1
  let last = 0
  for (const character of needle) {
    const found = haystack.indexOf(character, index)
    if (found === -1) return undefined
    if (first === -1) first = found
    last = found
    index = found + 1
  }
  return last - first + first
}
