/**
 * The address bar, as a task query.
 *
 * # The failure this module prevents
 *
 * Three views building the same filter three ways. The board, the list and My
 * Work all read the same URL and all have to turn it into the same
 * `GET /api/v1/tasks` call — and the moment one of them forgets `type` or spells
 * "unassigned" differently, a user switching views sees a different set of rows
 * for what the address says is the same question. That is the bug that makes
 * people stop trusting a filter.
 *
 * So the translation happens once, here, and a view adds only what is genuinely
 * its own — the board adds a status per column, My Work adds `@me`.
 *
 * # `assignee=` is not the same as no `assignee`
 *
 * An empty value is the grammar's `is_empty` (`docs/27` §URL form: "`field=` —
 * the empty value is how a URL says 'unset'"), so `?assignee=` means *unassigned*
 * and omitting it means *anyone*. Every other parameter drops when empty; this
 * one cannot, and that asymmetry is the reason this file exists rather than a
 * spread of `search` into `filter`.
 */
import type { TaskFilter } from '../api/tasks'
import type { AppSearch } from '../router'

/**
 * The filter a view should send for the current address.
 *
 * Absent parameters are simply absent from the result: the server refuses an
 * unknown *field*, but an omitted one is just an unconstrained one, and building
 * `{ priority: undefined }` would put `priority=undefined` on the wire.
 */
export function filterFromSearch(search: AppSearch): TaskFilter {
  const filter: Record<string, string> = {}

  if (search.project !== undefined) filter['project'] = search.project
  // Present-and-empty is the triage queue: owned by no team yet (`docs/45`).
  if (search.team !== undefined) filter['team'] = search.team
  if (search.q !== undefined && search.q !== '') filter['q'] = search.q
  if (search.status !== undefined && search.status !== '') filter['status'] = search.status
  if (search.priority !== undefined && search.priority !== '') filter['priority'] = search.priority
  if (search.type !== undefined && search.type !== '') filter['type'] = search.type
  if (search.due !== undefined && search.due !== '') filter['due_at'] = search.due
  if (search.reporter !== undefined && search.reporter !== '') filter['reporter'] = search.reporter
  if (search.state !== undefined && search.state !== '') filter['state'] = search.state
  if (search.created !== undefined && search.created !== '') filter['created_at'] = search.created
  if (search.updated !== undefined && search.updated !== '') filter['updated_at'] = search.updated
  if (search.title !== undefined && search.title !== '') filter['title'] = search.title
  if (search.archived !== undefined && search.archived !== '') filter['archived'] = search.archived
  if (search.blocked !== undefined && search.blocked !== '') filter['is_blocked'] = search.blocked
  // Present-and-empty is meaningful for these three — see the module docs.
  if (search.assignee !== undefined) filter['assignee'] = search.assignee
  if (search.tag !== undefined) filter['tag'] = search.tag
  if (search.parent !== undefined) filter['parent'] = search.parent

  return filter as TaskFilter
}

/**
 * Which address bar parameter carries each filter field.
 *
 * The two names differ in three places — `due_at`/`due`, `is_blocked`/`blocked`,
 * `created_at`/`created` — because `docs/27`'s grammar names database fields and
 * the address bar is written by hand. That mismatch is exactly the sort of thing
 * a second, hand-written copy gets wrong in one direction only, so both
 * directions read this one table.
 *
 * Typed as `Record<keyof TaskFilter, …>`, which makes it **exhaustive**: adding a
 * field to `TaskFilter` fails the build here until someone decides whether it
 * belongs in a URL. `null` means it deliberately does not — `key`, `milestone`
 * and `environment` have no search parameter, so a view cannot link to them and
 * this says so in the type rather than dropping them silently.
 */
const SEARCH_KEY_OF: Record<keyof TaskFilter, keyof AppSearch | null> = {
  project: 'project',
  team: 'team',
  q: 'q',
  status: 'status',
  priority: 'priority',
  type: 'type',
  due_at: 'due',
  reporter: 'reporter',
  state: 'state',
  created_at: 'created',
  updated_at: 'updated',
  title: 'title',
  archived: 'archived',
  is_blocked: 'blocked',
  assignee: 'assignee',
  tag: 'tag',
  parent: 'parent',
  key: null,
  milestone: null,
  environment: null,
}

/**
 * The address that shows exactly these tasks.
 *
 * The inverse of [`filterFromSearch`], and the reason a dashboard tile is
 * something you can *act on*: a number nobody can click through to is trivia.
 * "Overdue: 3" becomes a link to the three tasks, in the list, with the filter
 * already applied — the same list, reached the same way, so the count and the
 * rows cannot disagree.
 *
 * Empty values survive, because `field=` is the grammar's `is_empty`: a tile
 * counting unassigned work must link to `?assignee=`, not to everything.
 */
export function searchFromFilter(filter: TaskFilter): Partial<AppSearch> {
  const search: Record<string, string> = {}
  for (const [field, value] of Object.entries(filter)) {
    const key = SEARCH_KEY_OF[field as keyof TaskFilter]
    if (key === null || key === undefined || value === undefined) continue
    search[key] = value
  }
  return search as Partial<AppSearch>
}

/** Whether anything is narrowing the view, so "Clear" can hide when there is nothing to clear. */
export function hasFilters(search: AppSearch): boolean {
  return (
    (search.q !== undefined && search.q !== '') ||
    (search.status !== undefined && search.status !== '') ||
    (search.priority !== undefined && search.priority !== '') ||
    (search.type !== undefined && search.type !== '') ||
    (search.due !== undefined && search.due !== '') ||
    (search.reporter !== undefined && search.reporter !== '') ||
    (search.state !== undefined && search.state !== '') ||
    (search.created !== undefined && search.created !== '') ||
    (search.updated !== undefined && search.updated !== '') ||
    (search.title !== undefined && search.title !== '') ||
    (search.archived !== undefined && search.archived !== '') ||
    (search.blocked !== undefined && search.blocked !== '') ||
    search.assignee !== undefined ||
    search.tag !== undefined ||
    search.parent !== undefined
  )
}

/** Every filter parameter cleared at once, for "Clear" and for applying a view. */
export const NO_FILTERS: Readonly<Record<string, undefined>> = {
  q: undefined,
  status: undefined,
  priority: undefined,
  type: undefined,
  assignee: undefined,
  reporter: undefined,
  due: undefined,
  state: undefined,
  created: undefined,
  updated: undefined,
  title: undefined,
  tag: undefined,
  parent: undefined,
  archived: undefined,
  blocked: undefined,
}

/**
 * The due-date presets, in the grammar's own spelling.
 *
 * Relative symbols rather than computed dates: `docs/27` resolves `@today` in the
 * **actor's** offset, and a client that sent an absolute instant would compute
 * midnight against the browser's clock instead — "the classic and extremely
 * confusing bug" that document names. It also means a shared link still means
 * "this week" tomorrow, rather than freezing the week it was copied.
 */
export const DUE_PRESETS: readonly { readonly value: string; readonly label: string }[] = [
  { value: '', label: 'Any due date' },
  { value: '<@today', label: 'Overdue' },
  { value: '<@tomorrow', label: 'Due today or earlier' },
  { value: '<+7d', label: 'Due within a week' },
  { value: '<+30d', label: 'Due within a month' },
]
