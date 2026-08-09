/**
 * One task, rendered the same way everywhere it appears.
 *
 * # The failure this module prevents
 *
 * Three views drawing the same task three ways. The board, the list and My Work
 * all answer "what is this task and should I care?", and when each builds its own
 * markup they drift within a week: one shows the due date, one shows priority as
 * a word, one shows a grey pill reading "None" on every single row. A user who
 * learns to read one view then has to relearn the next.
 *
 * # What a card shows, and why that set
 *
 * Type, key, title, a description snippet, priority, and the due date. That is
 * everything `TaskView` carries that helps someone triage — and deliberately not
 * everything it carries. `updated_at`, `version` and the uuids answer questions
 * nobody asks while scanning a column.
 *
 * # `NONE` renders as nothing
 *
 * A priority pill reading "None" on most rows is pure noise: it occupies the
 * position where a signal would be, so the eye stops checking that position and
 * misses the URGENT. Absence *is* the display of `NONE`.
 */
import type { ReactElement } from 'react'

import type { Task } from '../api/tasks'
import { TaskLink } from '../task/TaskLink'
import { formatRelative, isOverdue, priorityLabel, typeLabel } from './present'

/** The longest description snippet a card carries before it stops being a card. */
const SNIPPET = 120

export function TaskCard({
  task,
  onOpen,
  handle,
}: {
  task: Task
  onOpen: (id: string) => void
  /** The drag grip, when the surface supports dragging. */
  handle?: ReactElement | null
}): ReactElement {
  return (
    <article className="card">
      {/* The title first.
          A card is read by its summary; the key and the type are how you refer
          to it *after* you have found it. Leading with a grey uppercase TASK
          chip meant every card in a column opened with the same word, and the
          eye had to step over it to reach the one line that differed. */}
      <div className="card__top">
        {/* An anchor, so ⌘-click opens the task in a tab from the board — the
            gesture people use to work on three cards at once. A plain click
            keeps the board where it is and opens the peek. */}
        <TaskLink taskId={task.id} className="card__title" onPeek={onOpen}>
          {task.title}
        </TaskLink>
        {handle}
      </div>

      {task.description === null || task.description.trim() === '' ? null : (
        <p className="card__snippet">{snippet(task.description)}</p>
      )}

      <div className="card__foot">
        <TypeBadge type={task.type} />
        <span className="key">{task.key}</span>
        <span className="card__grow" />
        <TaskMeta task={task} />
      </div>
    </article>
  )
}

/**
 * The signals under a title: priority and the due date.
 *
 * Shared with the list row, which needs the identical judgement about what is
 * worth showing — otherwise a task that looks urgent on the board looks ordinary
 * in the list.
 */
export function TaskMeta({ task }: { task: Task }): ReactElement | null {
  const overdue = isOverdue(task)
  const showPriority = task.priority !== 'NONE'
  if (!showPriority && task.due_at === null) return null

  return (
    <div className="card__signals">
      {showPriority ? <PriorityBadge priority={task.priority} /> : null}
      {task.due_at === null ? null : (
        <span className={overdue ? 'card__due card__due--late' : 'card__due'}>
          {/* `time` so the machine-readable instant travels with the human words:
              "in 20 days" is unusable to anyone whose reader announces the
              element rather than reading the sentence around it. */}
          <time dateTime={task.due_at}>
            {overdue ? 'overdue ' : 'due '}
            {formatRelative(task.due_at)}
          </time>
        </span>
      )}
    </div>
  )
}

/**
 * Priority, coloured by severity.
 *
 * The colours carry meaning, so the *word* is always present too — WCAG 2.2
 * §1.4.1: colour is never the only way information is conveyed. Someone with a
 * red-green deficiency reads "Urgent"; everyone else sees it before they read
 * anything.
 */
export function PriorityBadge({ priority }: { priority: string }): ReactElement {
  return <span className={`pill pill--priority-${priority}`}>{priorityLabel(priority)}</span>
}

/** The kind of work. A bug and a feature are scanned differently, so they look different. */
export function TypeBadge({ type }: { type: string }): ReactElement {
  return <span className={`badge badge--${type}`}>{typeLabel(type)}</span>
}

/**
 * The first line or so of a description.
 *
 * Cut on a word boundary and at the first blank line: a snippet that stops
 * mid-word reads as corrupted data, and one that runs across a paragraph break
 * glues two unrelated sentences together.
 */
function snippet(description: string): string {
  const firstBlock = description.split('\n\n')[0] ?? description
  const flat = firstBlock.replace(/\s+/g, ' ').trim()
  if (flat.length <= SNIPPET) return flat
  const cut = flat.slice(0, SNIPPET)
  const lastSpace = cut.lastIndexOf(' ')
  return `${cut.slice(0, lastSpace > 40 ? lastSpace : SNIPPET)}…`
}
