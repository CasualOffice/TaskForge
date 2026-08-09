/**
 * The built-in saved views, from `docs/27` §Built-in views.
 *
 * # Why these are transcribed rather than invented
 *
 * `docs/27` ships seven views and says why they are defined in the grammar
 * instead of as hand-written SQL: "If a built-in view needed a capability the
 * grammar lacks, that is the signal the grammar is under-specified." Writing
 * them here, in the URL form, is the client half of that check — and it found
 * two server bugs on the first run (see `unavailable` below).
 *
 * # The translation from the document's notation to the URL form
 *
 * `docs/27` §Built-in views writes filters in prose-ish algebra; §URL form fixes
 * the wire spelling. Two translations are not literal and both are exact:
 *
 * - **`due_at <= @today` becomes `due_at=<@tomorrow`.** `due_at` permits
 *   `before after between is_empty` and *not* `lte`
 *   (`casual-task-search/src/filter.rs::operators`), so `<=` has no spelling.
 *   "Before tomorrow" and "on or before today" are the same set of instants,
 *   and the document's own boundary is preserved.
 * - **`state not in (...)` becomes `state=!COMPLETED,CANCELED`**, which is the
 *   `field=!a,b` row of the §URL form table.
 *
 * # Two views are declared and withheld
 *
 * `My Work · Blocked` is transcribed and marked unavailable, because
 * `is_blocked` compiles to a column the schema does not have (`compile.rs`
 * emits `d.blocked_task_id`; `migrations/0005_tasks.sql` defines
 * `from_task_id` / `to_task_id`).
 *
 * `My Work · Upcoming` was withheld for the same reason until `resolve.rs`
 * learned to descend into a `between` clause's bounds; it is live now, which is
 * what keeping it here rather than deleting it was for. The list of what
 * TaskForge *means* to ship does not quietly shrink to what it currently can.
 */
import type { AppSearch } from '../router'

export interface BuiltinView {
  readonly id: string
  readonly name: string
  /** The filter, as the search parameters that express it. */
  readonly search: Partial<AppSearch>
  /** Set when the server cannot currently answer it; the reason is shown. */
  readonly unavailable?: string
  /** Views about the actor personally, which My Work groups separately. */
  readonly mine: boolean
}

export const BUILTIN_VIEWS: readonly BuiltinView[] = [
  {
    id: 'my-today',
    name: 'Today',
    mine: true,
    // assignee=@me AND state in (PLANNED,ACTIVE) AND due_at <= @today
    search: { assignee: '@me', state: 'PLANNED,ACTIVE', due: '<@tomorrow' },
  },
  {
    id: 'my-overdue',
    name: 'Overdue',
    mine: true,
    // assignee=@me AND state not in (COMPLETED,CANCELED) AND due_at < @today
    search: { assignee: '@me', state: '!COMPLETED,CANCELED', due: '<@today' },
  },
  {
    id: 'my-upcoming',
    name: 'Upcoming',
    mine: true,
    // assignee=@me AND due_at between @tomorrow..+14d
    search: { assignee: '@me', due: '@tomorrow..+14d' },
  },
  {
    id: 'my-blocked',
    name: 'Blocked',
    mine: true,
    // assignee=@me AND is_blocked=true
    search: { assignee: '@me', blocked: 'true' },
    unavailable:
      'The server returns TF-SYS-0001 for is_blocked — the compiler emits ' +
      'd.blocked_task_id and the schema has from_task_id / to_task_id.',
  },
  {
    id: 'my-recently-completed',
    name: 'Recently completed',
    mine: true,
    // assignee=@me AND state=COMPLETED AND updated_at > -7d
    search: { assignee: '@me', state: 'COMPLETED', updated: '>-7d' },
  },
  {
    id: 'reported-by-me',
    name: 'Reported by me',
    mine: true,
    // reporter=@me AND state not in (COMPLETED,CANCELED)
    search: { reporter: '@me', state: '!COMPLETED,CANCELED' },
  },
  {
    id: 'unassigned',
    name: 'Unassigned',
    mine: false,
    // assignee is_empty AND state in (BACKLOG,PLANNED)
    search: { assignee: '', state: 'BACKLOG,PLANNED' },
  },
]

/**
 * The view the current address is showing, if it is exactly one of them.
 *
 * Compared field by field rather than by serialising both sides: parameter order
 * in a URL is not meaningful, and a string comparison would call the same filter
 * two different views depending on which control the user touched last.
 *
 * `project` and `task` are excluded from the comparison — a built-in view scoped
 * to a project is still that view, and an open drawer is not part of the filter.
 */
export function activeView(search: AppSearch): BuiltinView | undefined {
  return BUILTIN_VIEWS.find((view) => matches(view.search, search))
}

function matches(view: Partial<AppSearch>, search: AppSearch): boolean {
  const keys = new Set([...Object.keys(view), ...FILTER_FIELDS])
  for (const key of keys) {
    if (!FILTER_FIELDS.includes(key)) continue
    const wanted = (view as Record<string, string | undefined>)[key]
    const actual = (search as Record<string, string | undefined>)[key]
    if (wanted !== actual) return false
  }
  return true
}

/** The parameters a view can constrain. Deliberately not `project` or `task`. */
const FILTER_FIELDS: readonly string[] = [
  'q',
  'status',
  'priority',
  'type',
  'assignee',
  'reporter',
  'due',
  'state',
  'created',
  'updated',
  'title',
  'tag',
  'parent',
  'archived',
  'blocked',
]
