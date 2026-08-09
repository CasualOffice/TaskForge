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
  if (search.q !== undefined && search.q !== '') filter['q'] = search.q
  if (search.status !== undefined && search.status !== '') filter['status'] = search.status
  if (search.priority !== undefined && search.priority !== '') filter['priority'] = search.priority
  if (search.type !== undefined && search.type !== '') filter['type'] = search.type
  if (search.due !== undefined && search.due !== '') filter['due_at'] = search.due
  if (search.reporter !== undefined && search.reporter !== '') filter['reporter'] = search.reporter
  // Present-and-empty is meaningful — see the module docs.
  if (search.assignee !== undefined) filter['assignee'] = search.assignee

  return filter as TaskFilter
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
    search.assignee !== undefined
  )
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
