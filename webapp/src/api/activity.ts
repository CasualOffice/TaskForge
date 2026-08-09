/**
 * `GET /api/v1/tasks/{id}/activity` — what happened to this task.
 *
 * # Why the server sends `changes` and not a sentence
 *
 * `docs/25`: an activity record stores **display values, not ids** — the status
 * *names* at the time, because either may be renamed or deleted before anyone
 * reads the record. What it does not store is a rendered sentence, and that is
 * deliberate: the wording is presentation, and a stored sentence would be frozen
 * in whatever language and phrasing existed the day it was written.
 *
 * So the client renders it. `describe()` below is that rendering, and it is
 * written to degrade rather than throw: an event type it has never seen is
 * reported as the event type, which is ugly and true, instead of crashing a
 * panel or silently dropping a row from the history.
 */
import { query, request } from './http'
import type { Paged } from './page'

export interface ActivityEntry {
  readonly id: string
  readonly event_type: string
  /** `null` for a system actor — a sweeper, a migration, the dispatcher. */
  readonly actor_id: string | null
  readonly actor_name: string | null
  /** Shape depends on `event_type`. Rendered by `describe`, never trusted. */
  readonly changes: Record<string, unknown>
  readonly occurred_at: string
}

export function readActivity(
  workspaceId: string,
  taskId: string,
  cursor?: string,
  signal?: AbortSignal,
): Promise<Paged<ActivityEntry>> {
  return request<Paged<ActivityEntry>>(
    `/api/v1/tasks/${taskId}/activity${query({ limit: 50, cursor })}`,
    { workspaceId, signal },
  )
}

/** A `{ from, to }` pair, when the change carries one. */
function transition(value: unknown): { from?: string; to?: string } | undefined {
  if (typeof value !== 'object' || value === null) return undefined
  const pair = value as { from?: unknown; to?: unknown }
  const from = typeof pair.from === 'string' ? pair.from : undefined
  const to = typeof pair.to === 'string' ? pair.to : undefined
  if (from === undefined && to === undefined) return undefined
  return { ...(from === undefined ? {} : { from }), ...(to === undefined ? {} : { to }) }
}

/**
 * One line of history, in the reader's language.
 *
 * Every branch reads the stored *display values* rather than looking anything
 * up: the record is the truth about what it looked like then, and resolving an
 * id now would show today's name against a year-old event.
 */
export function describe(entry: ActivityEntry): string {
  const changes = entry.changes
  switch (entry.event_type) {
    case 'task.created':
      return 'created this task'
    case 'task.deleted':
      return 'deleted this task'
    case 'task.reopened':
    case 'task.status.changed': {
      const status = transition(changes['status'])
      if (status === undefined) return 'changed the status'
      const reopened = entry.event_type === 'task.reopened'
      const verb = reopened ? 'reopened it' : 'moved it'
      if (status.from === undefined) return `${verb} to ${status.to ?? 'another status'}`
      return `${verb} from ${status.from} to ${status.to ?? 'another status'}`
    }
    case 'task.assigned':
      return 'changed who is assigned'
    case 'task.dependency.added':
      return 'added a dependency'
    case 'task.dependency.removed':
      return 'removed a dependency'
    case 'task.comment.added':
      return 'commented'
    case 'task.updated': {
      // The field names are the record's own keys. Listing them is more useful
      // than "updated the task" and does not require this file to know what
      // every field is called.
      const fields = Object.keys(changes)
      if (fields.length === 0) return 'updated this task'
      return `changed ${fields.join(', ')}`
    }
    default:
      // Honest rather than tidy: an unknown event still gets a row, and the row
      // says what it was. A dropped row is a hole in an audit trail.
      return entry.event_type
  }
}
