/**
 * Paging through tasks, one keyset at a time.
 *
 * # The failure this module prevents
 *
 * `OFFSET`, reintroduced from the client. ADR-011 and `docs/26` forbid it
 * server-side, and the server has no parameter for it — so the way it comes back
 * is a client that keeps a page *number*, multiplies it by a page size, and
 * either refetches from the top on every page or asks for `limit=1000` once.
 * Both destroy the guarantee the index exists for.
 *
 * `useInfiniteQuery` with the opaque `next_cursor` as its page parameter is the
 * whole mechanism: there is no page number in this file, no arithmetic on a
 * cursor, and `getNextPageParam` returns the server's own string or nothing.
 *
 * # Why the flattened rows are memoized
 *
 * The virtualizer measures against this array. Rebuilding it on every render
 * gives every row a new identity, which re-mounts the visible window and drops
 * the 60 fps target docs/42 sets for scrolling a 2,000-card board.
 */
import { useMemo } from 'react'
import { useInfiniteQuery, type UseInfiniteQueryResult } from '@tanstack/react-query'

import { keys } from '../api/keys'
import { MAX_LIMIT, nextCursor, type Paged } from '../api/page'
import { listTasks, type Task, type TaskQuery } from '../api/tasks'

export interface TaskFeed {
  readonly rows: readonly Task[]
  readonly isPending: boolean
  readonly error: unknown
  readonly hasMore: boolean
  readonly isFetchingMore: boolean
  readonly fetchMore: () => void
  readonly refetch: () => void
}

/**
 * A cursor-paged task list.
 *
 * `spec.cursor` is ignored if a caller sets it — the first page has no cursor by
 * definition, and accepting one here would let a view resume a keyset it did not
 * receive, which is how a "page 2" bookmark starts returning a different page 2
 * each week.
 */
export function useTaskFeed(
  workspaceId: string,
  spec: TaskQuery,
  options: { enabled?: boolean } = {},
): TaskFeed {
  const pageSize = Math.min(spec.limit ?? MAX_LIMIT, MAX_LIMIT)
  const stable = useMemo<TaskQuery>(
    () => ({ ...spec, limit: pageSize, cursor: undefined }),
    [spec, pageSize],
  )

  const result: UseInfiniteQueryResult<{ pages: Paged<Task>[] }, unknown> = useInfiniteQuery({
    queryKey: keys.taskList(workspaceId, stable),
    queryFn: ({ pageParam, signal }) =>
      listTasks(workspaceId, { ...stable, cursor: pageParam }, signal),
    initialPageParam: undefined as string | undefined,
    getNextPageParam: (last: Paged<Task>) => nextCursor(last.page),
    enabled: (options.enabled ?? true) && workspaceId !== '',
    // A list is server-authoritative and cheap to refetch; 10 s is long enough
    // that scrolling does not thrash the API and short enough that a colleague's
    // change shows up without a reload. SSE invalidation (C-015) will make this
    // number matter less, not more.
    staleTime: 10_000,
  })

  const rows = useMemo(
    () => (result.data?.pages ?? []).flatMap((page) => page.data),
    [result.data],
  )

  return {
    rows,
    isPending: result.isPending,
    error: result.error,
    hasMore: result.hasNextPage,
    isFetchingMore: result.isFetchingNextPage,
    fetchMore: () => {
      if (result.hasNextPage && !result.isFetchingNextPage) void result.fetchNextPage()
    },
    refetch: () => void result.refetch(),
  }
}
