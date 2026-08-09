/**
 * Reading and writing the shared search parameters.
 *
 * # The failure this module prevents
 *
 * Four views each spelling `?task=` for themselves. The parameter name is part
 * of every deep link a user has ever pasted into a ticket, so it is a contract;
 * one view typing `taskId` would produce links that open nothing, and the bug
 * would only show up when somebody clicked a colleague's URL.
 *
 * It also keeps the *merge* right: navigating with a partial search object must
 * preserve the parameters it does not mention, or opening a drawer would silently
 * drop the project filter the user had chosen.
 */
import { useCallback } from 'react'
import { useNavigate, useSearch } from '@tanstack/react-router'

import { EMPTY_IS_MEANINGFUL, type AppSearch } from '../router'

export function useAppSearch(): AppSearch {
  return useSearch({ strict: false }) as AppSearch
}

/** Merge a partial change into the current search, replacing rather than pushing. */
export function useUpdateSearch(): (next: Partial<AppSearch>) => void {
  const navigate = useNavigate()
  return useCallback(
    (next) => {
      void navigate({
        // `undefined` in `next` clears a parameter; anything absent is kept.
        //
        // The cast is the router's typing, not a hole in ours: `search` is typed
        // against the union of every route's own search shape, and every route
        // here inherits the root's `AppSearch`. `prune` still returns `AppSearch`.
        search: ((current: AppSearch) => prune({ ...current, ...next })) as never,
        // `replace`, so opening and closing a drawer does not fill the back
        // button with a history of the same view.
        replace: true,
      })
    },
    [navigate],
  )
}

/** Open the drawer on a task, or close it with `undefined`. */
export function useOpenTask(): (taskId: string | undefined) => void {
  const update = useUpdateSearch()
  return useCallback((taskId) => update({ task: taskId }), [update])
}

/**
 * Drop empty values so the URL never carries `?task=`.
 *
 * The exceptions are not special cases so much as the grammar's own rule:
 * `docs/27` §URL form says "`field=` — the empty value is how a URL says
 * 'unset'", so `?assignee=` means *unassigned*, `?team=` means *untriaged*, and
 * dropping either would silently widen the filter instead of narrowing it. The
 * list is `EMPTY_IS_MEANINGFUL`, shared with the router's validator, because
 * two copies of it drifted once already.
 *
 * Exported for the guard test that pins that sharing — the drift it prevents is
 * invisible until someone tries to filter by "untagged" and touches the page.
 */
export function prune(search: AppSearch): AppSearch {
  const out: Record<string, string> = {}
  for (const [name, value] of Object.entries(search)) {
    if (typeof value !== 'string') continue
    if (value === '' && !EMPTY_IS_MEANINGFUL.has(name)) continue
    out[name] = value
  }
  return out as AppSearch
}
