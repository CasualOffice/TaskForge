/**
 * The column a list is ordered by.
 *
 * # The failure this module prevents
 *
 * Offering a sort the server refuses. `docs/26` closes the sortable set and
 * makes it **smaller** than the filterable one — `title` filters through a
 * trigram index but ordering by it has nothing behind it, and `rank` is only
 * meaningful beside a `q` clause. A header wired to a field outside that set is
 * a `TF-QRY-0002` the user triggered by clicking a column.
 *
 * So the type is `SortKey` from `api/tasks.ts`, which is the closed set, and the
 * default matches the server's own (`updated_at` descending) — a client whose
 * default disagreed would reorder the list on first interaction for no reason
 * the user could see.
 */
import { useCallback, useState } from 'react'

import type { Sort } from '../api/tasks'

/** `Sort::default()` in `casual-task-search/src/sort.rs`: newest first. */
export const DEFAULT_SORT: Sort = { key: 'updated_at', descending: true }

/**
 * The current ordering.
 *
 * Component state rather than a URL parameter, deliberately: `sort` is a
 * *reading* preference, not part of what the view is about, and putting it in
 * the address would make two links to the same filtered list compare unequal.
 * The scope — project and search term — is in the URL for the opposite reason.
 */
export function useSortPreference(initial: Sort = DEFAULT_SORT): [Sort, (next: Sort) => void] {
  const [sort, setSort] = useState<Sort>(initial)
  const change = useCallback((next: Sort) => setSort(next), [])
  return [sort, change]
}
