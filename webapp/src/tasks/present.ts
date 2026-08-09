/**
 * How a task field looks on screen.
 *
 * # The failure this module prevents
 *
 * The same field rendered three different ways in three views. A due date that
 * reads "in 2 days" on a card, "2026-08-11T00:00:00Z" in the drawer, and
 * "11/08/2026" in the list is not three styles — it is three chances for a user
 * to read the wrong day, and one of them will be ambiguous about which number is
 * the month.
 *
 * # `Intl`, and nothing else
 *
 * docs/42 §Rules that keep it that way: "One date library. `Intl` where it
 * suffices." It suffices for everything here, so the date-formatting cost of
 * this app is zero bytes.
 */
import type { Priority, Task, TaskState, TaskType } from '../api/tasks'

/** The five permanent states, as a person reads them (`docs/23`). */
const STATE_LABELS: Readonly<Record<TaskState, string>> = {
  BACKLOG: 'Backlog',
  PLANNED: 'Planned',
  ACTIVE: 'Active',
  COMPLETED: 'Completed',
  CANCELED: 'Canceled',
}

const TYPE_LABELS: Readonly<Record<TaskType, string>> = {
  TASK: 'Task',
  BUG: 'Bug',
  FEATURE: 'Feature',
  INCIDENT: 'Incident',
  REQUEST: 'Request',
}

const PRIORITY_LABELS: Readonly<Record<Priority, string>> = {
  NONE: 'None',
  LOW: 'Low',
  MEDIUM: 'Medium',
  HIGH: 'High',
  URGENT: 'Urgent',
}

export function stateLabel(state: string): string {
  return STATE_LABELS[state as TaskState] ?? state
}

export function typeLabel(type: string): string {
  return TYPE_LABELS[type as TaskType] ?? type
}

export function priorityLabel(priority: string): string {
  return PRIORITY_LABELS[priority as Priority] ?? priority
}

const DATE = new Intl.DateTimeFormat(undefined, { dateStyle: 'medium' })
const DATE_TIME = new Intl.DateTimeFormat(undefined, { dateStyle: 'medium', timeStyle: 'short' })

/** A date, in the reader's locale. `—` for absent, never an empty cell. */
export function formatDate(iso: string | null): string {
  if (iso === null) return '—'
  const at = new Date(iso)
  return Number.isNaN(at.getTime()) ? iso : DATE.format(at)
}

export function formatDateTime(iso: string | null): string {
  if (iso === null) return '—'
  const at = new Date(iso)
  return Number.isNaN(at.getTime()) ? iso : DATE_TIME.format(at)
}

/**
 * "3 days ago". Falls back to the absolute date beyond a month.
 *
 * Relative time is easier to read for recent things and *harder* for old ones —
 * "11 months ago" tells a reader less than "Sep 2025", and it is the older rows
 * where someone is trying to establish exactly when something happened.
 */
const RELATIVE = new Intl.RelativeTimeFormat(undefined, { numeric: 'auto' })
const MINUTE = 60_000
const HOUR = 60 * MINUTE
const DAY = 24 * HOUR

export function formatRelative(iso: string | null, now = Date.now()): string {
  if (iso === null) return '—'
  const at = new Date(iso).getTime()
  if (Number.isNaN(at)) return iso
  const delta = at - now
  const magnitude = Math.abs(delta)
  if (magnitude < MINUTE) return 'just now'
  if (magnitude < HOUR) return RELATIVE.format(Math.round(delta / MINUTE), 'minute')
  if (magnitude < DAY) return RELATIVE.format(Math.round(delta / HOUR), 'hour')
  if (magnitude < 30 * DAY) return RELATIVE.format(Math.round(delta / DAY), 'day')
  return formatDate(iso)
}

/** Whether a due date has passed and the task is not finished. */
export function isOverdue(task: Task, now = Date.now()): boolean {
  if (task.due_at === null) return false
  if (task.state === 'COMPLETED' || task.state === 'CANCELED') return false
  const due = new Date(task.due_at).getTime()
  return !Number.isNaN(due) && due < now
}

/**
 * The value a `<input type="date">` wants, from an RFC 3339 instant.
 *
 * Sliced rather than formatted: `toISOString()` would convert to UTC and shift
 * the date by a day for anyone east or west of it, which is the classic
 * off-by-one that makes a due date land on the wrong day for half the world.
 */
export function toDateInput(iso: string | null): string {
  return iso === null ? '' : iso.slice(0, 10)
}

/** The inverse: a `YYYY-MM-DD` field back to the RFC 3339 the API accepts. */
export function fromDateInput(value: string): string | null {
  return value === '' ? null : `${value}T00:00:00Z`
}
