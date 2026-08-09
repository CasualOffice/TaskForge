/**
 * Which component renders which registry panel.
 *
 * # The failure this module prevents
 *
 * A drawer whose sections are a JSX list. `docs/34` and C-017 require the core's
 * own panels to render *through* the extension registry — otherwise the seam is
 * never exercised, and the first third-party panel discovers the drawer has
 * nowhere to put it. So the drawer iterates `contributions('ui.task.panel')` and
 * this table says what each slug draws.
 *
 * # A slug with no component is rendered, not skipped
 *
 * `attachments`, `relations` and `activity` are registered by the core and have
 * no HTTP surface yet. Skipping them would make the registry decorative — the
 * drawer would look identical whether the contribution existed or not. Rendering
 * the declared title with the reason underneath keeps the registry load-bearing
 * *and* tells the reader which half is missing.
 */
import type { ReactElement } from 'react'

import type { Task } from '../api/tasks'
import type { Authority } from '../shell/permissions'
import { GapNotice } from '../shell/notice'
import { formatDateTime } from '../tasks/present'
import { Assignees } from './Assignees'
import { CommentThread } from './CommentThread'
import { TaskFields } from './TaskFields'

export interface PanelProps {
  readonly task: Task
  readonly authority: Authority
}

/** The panels this client implements, by registry slug. */
const IMPLEMENTED: Readonly<Record<string, (props: PanelProps) => ReactElement>> = {
  details: DetailsPanel,
  comments: CommentsPanel,
}

/** Why a registered panel has nothing behind it, by slug. */
const UNBUILT: Readonly<Record<string, { tracker: string; because: string }>> = {
  attachments: {
    tracker: 'C-010',
    because:
      'The pipeline exists in casual-task-attachment — policy, sniffing, and the object-store ' +
      'seam — and no route is registered for POST /api/v1/tasks/{id}/attachments.',
  },
  relations: {
    tracker: 'C-008',
    because:
      'docs/05 specifies POST /api/v1/tasks/{id}/dependencies, cycle-checked. No route is ' +
      'registered for it and no endpoint reads a task’s relations.',
  },
  activity: {
    tracker: 'C-011',
    because:
      'Every change already writes an activity record in the same transaction as the change ' +
      'itself. GET /api/v1/tasks/{id}/activity is specified in docs/05 and not served.',
  },
}

/** The component for a slug, or `undefined` when the client has none. */
export function panelFor(slug: string): ((props: PanelProps) => ReactElement) | undefined {
  return IMPLEMENTED[slug]
}

export function unbuiltReason(slug: string): { tracker: string; because: string } | undefined {
  return UNBUILT[slug]
}

function DetailsPanel({ task, authority }: PanelProps): ReactElement {
  return (
    <>
      <TaskFields task={task} authority={authority} />
      <Assignees task={task} authority={authority} />
      <dl className="meta">
        <dt>Created</dt>
        <dd>{formatDateTime(task.created_at)}</dd>
        <dt>Updated</dt>
        <dd>{formatDateTime(task.updated_at)}</dd>
        <dt>Reporter</dt>
        <dd className="key">{task.reporter_id}</dd>
        <dt>Version</dt>
        <dd>{task.version}</dd>
      </dl>
    </>
  )
}

function CommentsPanel({ task, authority }: PanelProps): ReactElement {
  return <CommentThread taskId={task.id} authority={authority} />
}

/** The body of a registered panel with nothing behind it. */
export function UnbuiltPanel({ slug }: { slug: string }): ReactElement {
  const reason = unbuiltReason(slug)
  return (
    <GapNotice
      what="Registered in the extension point registry; nothing serves it yet."
      tracker={reason?.tracker ?? 'C-017'}
    >
      <span>{reason?.because ?? 'This client has no component for that contribution.'}</span>
    </GapNotice>
  )
}
