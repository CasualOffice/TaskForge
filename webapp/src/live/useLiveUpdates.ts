/**
 * Live updates: one stream per project, however many tabs are open.
 *
 * # The failure this module prevents
 *
 * N tabs meaning N streams. docs/42 §Live updates: "One SSE connection per
 * workspace, shared across tabs via `BroadcastChannel` — N tabs must not mean N
 * streams." A developer with six tabs open is not the pathological case; it is
 * Tuesday, and each stream costs the server a subscription and a revalidation
 * task for the life of the connection.
 *
 * Leader election is `navigator.locks`, not a heartbeat in `localStorage`. The
 * Web Locks API releases a lock when its tab dies — including a crash, a kill,
 * and a closed laptop — which is the case a hand-rolled lease gets wrong and
 * only notices as a stream nobody is reading. The follower tabs hear the same
 * invalidations over a `BroadcastChannel`.
 *
 * # Events invalidate; they do not patch
 *
 * docs/42: "Incoming events invalidate the relevant query keys rather than
 * patching the cache directly; TanStack Query then refetches only what is
 * mounted." Patching would mean this file owning a second, partial copy of every
 * server rule — what a transition does to `state`, what a delete does to a list
 * — and the day the two disagree, the visible one is wrong.
 *
 * # `stream.gap` refetches wholesale
 *
 * `docs/05`: past the replay window "the client is told to refetch rather than
 * being handed a partial history it would silently treat as complete." So a gap
 * clears the tenant prefix, not one task.
 */
import { useEffect } from 'react'
import { useQueryClient, type QueryClient } from '@tanstack/react-query'

import { keys } from '../api/keys'
import { aggregateIdOf, readStream, type LiveEvent } from './stream'

/** What a follower tab receives from the leader. */
interface Relayed {
  readonly workspaceId: string
  readonly taskId: string | undefined
  readonly gap: boolean
}

const CHANNEL = 'taskforge.live'

/** Back-off after a stream ends, so a refused connection does not spin. */
const RETRY_MS = 3_000

/**
 * Subscribe to a project's live events for as long as the component is mounted.
 *
 * Does nothing without a project: `docs/05` requires `project_id` and the server
 * refuses a wildcard subscription deliberately — "a wildcard subscription is one
 * refactor away from being the default, and its blast radius is every event in
 * the tenant."
 */
export function useLiveUpdates(workspaceId: string, projectId: string | undefined): void {
  const client = useQueryClient()

  useEffect(() => {
    if (workspaceId === '' || projectId === undefined) return

    const channel = new BroadcastChannel(CHANNEL)
    const controller = new AbortController()
    let stopped = false

    channel.onmessage = (message: MessageEvent<Relayed>) => {
      // A tab showing another workspace must ignore this one's events, or a
      // background tab refetches a tenant nobody is looking at.
      if (message.data.workspaceId !== workspaceId) return
      applyInvalidation(client, message.data)
    }

    function handle(event: LiveEvent): void {
      const relayed: Relayed = {
        workspaceId,
        taskId: aggregateIdOf(event),
        gap: event.type === 'stream.gap',
      }
      applyInvalidation(client, relayed)
      // The leader relays rather than each tab streaming. Followers apply the
      // identical invalidation, so a background tab is as fresh as the front one
      // without a second subscription.
      channel.postMessage(relayed)
    }

    // The lock is held for the life of the stream; a second tab's request waits
    // here rather than opening its own connection, and takes over the moment
    // this tab goes away for any reason at all.
    void navigator.locks.request(`${CHANNEL}.${projectId}`, { signal: controller.signal }, async () => {
      let lastEventId: string | undefined
      while (!stopped) {
        try {
          await readStream({
            workspaceId,
            projectId,
            lastEventId,
            signal: controller.signal,
            onEvent: (event) => {
              if (event.id !== undefined) lastEventId = event.id
              handle(event)
            },
          })
        } catch {
          // A refused or dropped connection. Not surfaced to the user: live
          // updates are an improvement on polling, and an error banner for a
          // background socket would train people to dismiss banners.
          if (stopped) return
        }
        if (stopped) return
        await new Promise((resolve) => setTimeout(resolve, RETRY_MS))
      }
    })

    return () => {
      stopped = true
      controller.abort()
      channel.close()
    }
  }, [client, workspaceId, projectId])
}

function applyInvalidation(client: QueryClient, relayed: Relayed): void {
  if (relayed.gap) {
    void client.invalidateQueries({ queryKey: keys.workspace(relayed.workspaceId) })
    return
  }
  // Lists always: a create, a delete and a transition all change which rows
  // belong in which list, and none of them is visible from the task's own key.
  void client.invalidateQueries({ queryKey: keys.taskLists(relayed.workspaceId) })
  if (relayed.taskId !== undefined) {
    void client.invalidateQueries({ queryKey: keys.task(relayed.workspaceId, relayed.taskId) })
  }
}
