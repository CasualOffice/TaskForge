/**
 * Task writes, applied optimistically and rolled back honestly.
 *
 * # The three rules docs/42 §Optimistic mutation calls not optional
 *
 * 1. **Every optimistic update carries a rollback token.** `onMutate` snapshots
 *    every cache entry it touches and returns it; `onError` puts it back. Not "a
 *    refetch on error" — a refetch is a *round trip* during which the user is
 *    looking at a lie.
 * 2. **`409` is handled, not thrown at the user.** A version conflict is the
 *    ordinary outcome of two people editing one task, so it produces the
 *    conflict remedy from `api/problem.ts` and a re-read, not a red box saying
 *    `TF-CNC-0001`.
 * 3. **Failure never discards user input.** Drafts live in the component that
 *    owns them and are cleared by `onSuccess`, never by `onMutate`. A comment
 *    that vanishes because the network blipped is the difference between a blip
 *    and a betrayal.
 *
 * # Why the list caches are patched and not just invalidated
 *
 * Invalidation is correct and slow: the card would sit in its old column until
 * the refetch lands. Both happen — the visible caches are patched now, and
 * `onSettled` invalidates so the server's answer is what survives.
 */
import { useMutation, useQueryClient, type QueryClient } from '@tanstack/react-query'

import { keys } from '../api/keys'
import type { Paged } from '../api/page'
import { patchTask, transitionTask, type Task, type TaskPatch } from '../api/tasks'

/** Every cache entry an optimistic update overwrote, with its previous value. */
type Rollback = readonly (readonly [readonly unknown[], unknown])[]

/** The shape `useInfiniteQuery` stores. Patched in place so pages keep their order. */
interface InfiniteTasks {
  readonly pages: readonly Paged<Task>[]
  readonly pageParams: readonly unknown[]
}

/**
 * Apply `next` to one task everywhere it is cached, returning the rollback token.
 *
 * Touches the detail entry and every list page. Both, because a task open in the
 * drawer is usually also a row behind it, and updating one of the two produces
 * an interface that contradicts itself while the user watches.
 */
function applyEverywhere(
  client: QueryClient,
  workspaceId: string,
  taskId: string,
  next: (task: Task) => Task,
): Rollback {
  const snapshots: (readonly [readonly unknown[], unknown])[] = []

  const detailKey = keys.task(workspaceId, taskId)
  const detail = client.getQueryData<Task>(detailKey)
  if (detail !== undefined) {
    snapshots.push([detailKey, detail])
    client.setQueryData(detailKey, next(detail))
  }

  for (const query of client.getQueryCache().findAll({ queryKey: keys.taskLists(workspaceId) })) {
    const data = query.state.data as InfiniteTasks | Paged<Task> | undefined
    if (data === undefined || !('pages' in data)) continue
    let touched = false
    const pages = data.pages.map((page) => {
      if (!page.data.some((row) => row.id === taskId)) return page
      touched = true
      return { ...page, data: page.data.map((row) => (row.id === taskId ? next(row) : row)) }
    })
    if (!touched) continue
    snapshots.push([query.queryKey, data])
    client.setQueryData(query.queryKey, { ...data, pages })
  }

  return snapshots
}

function restore(client: QueryClient, rollback: Rollback | undefined): void {
  for (const [key, value] of rollback ?? []) client.setQueryData(key, value)
}

export interface PatchVariables {
  readonly task: Task
  readonly patch: TaskPatch
}

/**
 * Edit a task's plain fields.
 *
 * The whole `task` is a variable rather than an id plus a version, because the
 * optimistic value needs the current row anyway and passing them separately is
 * how a call site ends up sending one task's version with another's id.
 */
export function useTaskPatch(workspaceId: string) {
  const client = useQueryClient()

  return useMutation<Task, unknown, PatchVariables, Rollback>({
    mutationFn: ({ task, patch }) => patchTask(workspaceId, task.id, task.version, patch),
    onMutate: async ({ task, patch }) => {
      await client.cancelQueries({ queryKey: keys.task(workspaceId, task.id) })
      // `version` is deliberately NOT advanced here. The server owns it, and
      // guessing `version + 1` would make the next write send a version that
      // never existed — turning a successful edit into a 409 the user caused by
      // typing quickly.
      return applyEverywhere(client, workspaceId, task.id, (current) => ({
        ...current,
        ...stripUndefined(patch),
      }))
    },
    onError: (_error, _variables, rollback) => restore(client, rollback),
    onSuccess: (updated) => {
      client.setQueryData(keys.task(workspaceId, updated.id), updated)
    },
    onSettled: (_data, _error, { task }) => {
      void client.invalidateQueries({ queryKey: keys.task(workspaceId, task.id) })
      void client.invalidateQueries({ queryKey: keys.taskLists(workspaceId) })
    },
  })
}

export interface TransitionVariables {
  readonly task: Task
  readonly toStatusId: string
  /** The state the target status maps onto, for the optimistic column move. */
  readonly toState: string
  readonly comment?: string
}

/**
 * Move a task through the workflow.
 *
 * The optimistic value sets both `status_id` and `state`, because the board
 * groups by `state` and the drawer shows `status_id`: updating one of the two
 * would move the card and leave the drawer claiming it had not moved.
 */
export function useTaskTransition(workspaceId: string) {
  const client = useQueryClient()

  return useMutation<Task, unknown, TransitionVariables, Rollback>({
    mutationFn: ({ task, toStatusId, comment }) =>
      transitionTask(workspaceId, task.id, task.version, {
        to_status_id: toStatusId,
        ...(comment === undefined ? {} : { comment }),
      }),
    onMutate: async ({ task, toStatusId, toState }) => {
      await client.cancelQueries({ queryKey: keys.task(workspaceId, task.id) })
      return applyEverywhere(client, workspaceId, task.id, (current) => ({
        ...current,
        status_id: toStatusId,
        state: toState as Task['state'],
      }))
    },
    onError: (_error, _variables, rollback) => restore(client, rollback),
    onSuccess: (updated) => {
      client.setQueryData(keys.task(workspaceId, updated.id), updated)
    },
    onSettled: (_data, _error, { task }) => {
      void client.invalidateQueries({ queryKey: keys.task(workspaceId, task.id) })
      void client.invalidateQueries({ queryKey: keys.taskLists(workspaceId) })
    },
  })
}

/**
 * Drop keys whose value is `undefined`.
 *
 * `{ description: undefined }` spread over a task would erase the description in
 * the optimistic copy and then have it reappear when the server answered. The
 * API distinguishes absent from `null` for the same reason (`docs/05`
 * §Conventions), and `null` is preserved here because it means "clear it".
 */
function stripUndefined(patch: TaskPatch): Partial<Task> {
  const out: Record<string, unknown> = {}
  for (const [name, value] of Object.entries(patch)) {
    if (value !== undefined) out[name] = value
  }
  return out as Partial<Task>
}
