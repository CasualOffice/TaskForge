/**
 * Turning a URL filter back into a sentence.
 *
 * # The failure this module prevents
 *
 * A chip that reads `state=!COMPLETED,CANCELED`. The URL carries the server's
 * grammar on purpose — one spelling, no client dialect (`docs/27` §Compilation)
 * — and that spelling is precise and unreadable. Showing it raw would make the
 * active-filter row an encoding puzzle, and hiding the row would bring back the
 * "why is this empty?" problem it exists to solve.
 *
 * So this is the one place the grammar is read *backwards*, into English. It is
 * presentation only: nothing here parses a filter in order to send it, so a gap
 * in this file makes a chip read poorly and can never make a query wrong.
 */
import type { AppSearch } from '../../router'
import { DUE_PRESETS } from '../../tasks/query'
import { priorityLabel, stateLabel, typeLabel } from '../../tasks/present'
import type { ActiveFilter } from './ActiveFilters'

/** Everything needed to turn an id into a name. */
export interface Labels {
  readonly status: (id: string) => string
  readonly person: (id: string) => string
}

/** The constraints in force, in the order they read most naturally. */
export function describeFilters(search: AppSearch, labels: Labels): ActiveFilter[] {
  const out: ActiveFilter[] = []
  const add = (key: keyof AppSearch, field: string, value: string): void => {
    out.push({ key, field, value })
  }

  if (search.q !== undefined && search.q !== '') add('q', 'Search', `“${search.q}”`)
  if (search.title !== undefined && search.title !== '') {
    add('title', 'Title contains', `“${search.title}”`)
  }
  if (search.status !== undefined && search.status !== '') {
    add('status', 'Status', list(search.status, labels.status))
  }
  if (search.state !== undefined && search.state !== '') {
    add('state', 'State', negatable(search.state, stateLabel))
  }
  if (search.priority !== undefined && search.priority !== '') {
    add('priority', 'Priority', comparable(search.priority, priorityLabel))
  }
  if (search.type !== undefined && search.type !== '') {
    add('type', 'Type', negatable(search.type, typeLabel))
  }
  if (search.assignee !== undefined) {
    add('assignee', 'Assignee', person(search.assignee, labels.person, 'Unassigned'))
  }
  if (search.reporter !== undefined && search.reporter !== '') {
    add('reporter', 'Reporter', person(search.reporter, labels.person, 'Nobody'))
  }
  if (search.due !== undefined && search.due !== '') add('due', 'Due', date(search.due))
  if (search.created !== undefined && search.created !== '') {
    add('created', 'Created', date(search.created))
  }
  if (search.updated !== undefined && search.updated !== '') {
    add('updated', 'Updated', date(search.updated))
  }
  if (search.tag !== undefined) add('tag', 'Tag', search.tag === '' ? 'Untagged' : search.tag)
  if (search.parent !== undefined) {
    add('parent', 'Nesting', search.parent === '' ? 'Top-level only' : 'Subtasks only')
  }
  if (search.archived === 'true') add('archived', 'Archived', 'Included')
  if (search.blocked !== undefined && search.blocked !== '') {
    add('blocked', 'Blocked', search.blocked === 'true' ? 'Yes' : 'No')
  }

  return out
}

/** `a,b` → "A or B". */
function list(raw: string, label: (value: string) => string): string {
  const parts = raw.split(',').map(label)
  if (parts.length === 1) return parts[0] ?? raw
  return `${parts.slice(0, -1).join(', ')} or ${parts.at(-1)}`
}

/** `!a,b` → "not A or B" (the grammar's `not_in`). */
function negatable(raw: string, label: (value: string) => string): string {
  const negated = raw.startsWith('!')
  const body = list(negated ? raw.slice(1) : raw, label)
  return negated ? `not ${body}` : body
}

/** `>=HIGH` → "High or above". `priority` is the one ordered enum (`docs/22`). */
function comparable(raw: string, label: (value: string) => string): string {
  if (raw.startsWith('>=')) return `${label(raw.slice(2))} or above`
  if (raw.startsWith('<=')) return `${label(raw.slice(2))} or below`
  if (raw.startsWith('>')) return `above ${label(raw.slice(1))}`
  if (raw.startsWith('<')) return `below ${label(raw.slice(1))}`
  return negatable(raw, label)
}

function person(raw: string, label: (id: string) => string, whenEmpty: string): string {
  if (raw === '') return whenEmpty
  if (raw === '@me') return 'Me'
  return label(raw)
}

/**
 * A date clause, preferring the preset's own wording.
 *
 * The presets are what most filters were built from, so matching them first
 * means the chip reads back exactly the phrase the user picked rather than a
 * re-derivation of it.
 */
function date(raw: string): string {
  const preset = DUE_PRESETS.find((option) => option.value === raw)
  if (preset !== undefined) return preset.label
  if (raw.includes('..')) {
    const [from, to] = raw.split('..')
    return `${symbol(from ?? '')} to ${symbol(to ?? '')}`
  }
  if (raw.startsWith('<')) return `before ${symbol(raw.slice(1))}`
  if (raw.startsWith('>')) return `after ${symbol(raw.slice(1))}`
  return symbol(raw)
}

/** `@today`, `+7d`, `-3mo` in words. Unknown spellings pass through unchanged. */
function symbol(raw: string): string {
  if (raw === '@today') return 'today'
  if (raw === '@tomorrow') return 'tomorrow'
  if (raw === '@start_of_week') return 'the start of this week'
  const relative = /^([+-])(\d+)(d|w|mo)$/.exec(raw)
  if (relative !== null) {
    const [, sign, count, unit] = relative
    const units = { d: 'day', w: 'week', mo: 'month' }[unit ?? 'd'] ?? 'day'
    const plural = count === '1' ? units : `${units}s`
    return sign === '+' ? `in ${count} ${plural}` : `${count} ${plural} ago`
  }
  return raw
}
