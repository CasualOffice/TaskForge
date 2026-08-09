/**
 * The board.
 *
 * # Why the columns are the five permanent states, not the workflow's statuses
 *
 * `docs/23` fixes five permanent states — `BACKLOG`, `PLANNED`, `ACTIVE`,
 * `COMPLETED`, `CANCELED` — and every `TaskView` carries the one it is in,
 * derived from its status in the same statement so the two cannot disagree.
 * Statuses are workspace-authored, and today a browser cannot read them at all:
 * `GET /api/v1/workflows/{id}` is specified in `docs/05` and not served
 * (**D-061**). Columns keyed on the closed set are therefore the only grouping
 * that is correct for every workspace *and* stable when the endpoint lands —
 * status columns become a refinement, not a rewrite.
 *
 * The cost, stated: a workspace with three statuses inside `ACTIVE` sees them in
 * one column. That is the losing side of this trade and it is real.
 *
 * # Why a drop can be refused before it is sent
 *
 * `POST /tasks/{id}/transitions` needs a `to_status_id`, and without the
 * workflow there is no way to learn one. Sending the move anyway would produce a
 * `400` the user cannot act on and a card that slides back for no visible
 * reason. So the cards are not draggable at all, and the reason is on screen.
 */
import { useCallback, useMemo, type ReactElement } from 'react'
import {
  DndContext,
  KeyboardSensor,
  PointerSensor,
  closestCenter,
  useSensor,
  useSensors,
  type Announcements,
  type DragEndEvent,
} from '@dnd-kit/core'
import { useQueryClient } from '@tanstack/react-query'

import { keys } from '../api/keys'
import { PERMISSIONS } from '../api/permissions'
import { TASK_STATES, type Task, type TaskState } from '../api/tasks'
import { TaskDrawer } from '../drawer/TaskDrawer'
import { useAnnounce } from '../shell/announce'
import { useAppSearch, useOpenTask } from '../shell/navigation'
import { ErrorNotice, GapNotice } from '../shell/notice'
import { useAuthority } from '../shell/permissions'
import { useWorkspaceId } from '../shell/session'
import { useLiveUpdates } from '../live/useLiveUpdates'
import { useTaskTransition } from '../tasks/mutations'
import { stateLabel } from '../tasks/present'
import { useProjectWorkflow } from '../tasks/useWorkflow'
import { BoardColumn } from './board/BoardColumn'
import { CreateTask } from './CreateTask'
import { ScopeBar } from './ScopeBar'

export function BoardView(): ReactElement {
  const workspaceId = useWorkspaceId()
  const search = useAppSearch()
  const openTask = useOpenTask()
  const announce = useAnnounce()
  const client = useQueryClient()

  const { unavailable: missingWorkflow, moveInto } = useProjectWorkflow(search.project)
  const authority = useAuthority(search.project)
  const move = useTaskTransition(workspaceId)

  // Live updates are project-scoped because the stream is: docs/05 requires
  // `project_id` and refuses a wildcard subscription deliberately.
  useLiveUpdates(workspaceId, search.project)

  // Two different reasons a card cannot move, kept apart. `docs/04` resolves the
  // second; hiding it behind the first would tell someone without
  // `task.transition` that the server is incomplete.
  const unavailable = authority.can(PERMISSIONS.taskTransition)
    ? missingWorkflow
    : 'You do not have permission to change the status of tasks here.'
  const canMove = unavailable === undefined

  const sensors = useSensors(
    useSensor(PointerSensor, {
      // A few pixels of slop, so a click on a card's title is a click and not a
      // one-pixel drag that swallows it.
      activationConstraint: { distance: 4 },
    }),
    useSensor(KeyboardSensor),
  )

  /** dnd-kit speaks these to a live region; without them a keyboard drag is silent. */
  const announcements = useMemo<Announcements>(
    () => ({
      onDragStart: ({ active }) => `Picked up ${String(active.id)}.`,
      onDragOver: ({ over }) =>
        over === null ? 'No column.' : `Over ${stateLabel(String(over.id))}.`,
      onDragEnd: ({ over }) =>
        over === null ? 'Move cancelled.' : `Dropped in ${stateLabel(String(over.id))}.`,
      onDragCancel: () => 'Move cancelled.',
    }),
    [],
  )

  const onDragEnd = useCallback(
    (event: DragEndEvent) => {
      const { active, over } = event
      if (over === null) return
      const toState = String(over.id) as TaskState
      const fromState = (active.data.current as { state?: string } | undefined)?.state
      if (fromState === toState) return

      // The card being dragged is the one already in a list cache; reading it
      // from there rather than refetching keeps the optimistic update instant.
      const task = findCachedTask(client, workspaceId, String(active.id))
      if (task === undefined) return

      // The workflow decides, not the column. `docs/23`: a transition exists or
      // it does not, and a board that inferred one from two states being
      // adjacent would send moves the state machine refuses.
      const edge = moveInto(task.status_id, toState)
      if (edge === undefined) {
        announce(`This workflow has no move from here into ${stateLabel(toState)}.`)
        return
      }
      if (!edge.permitted) {
        // Refused here rather than by the server, so the card never appears to
        // move and then springs back — which reads as a bug, not as a refusal.
        announce(`That move needs ${edge.needed ?? 'a permission you do not have'}.`)
        return
      }

      move.mutate(
        { task, toStatusId: edge.toStatusId, toState: edge.toState },
        { onSuccess: () => announce(`${task.key} moved to ${stateLabel(toState)}`) },
      )
    },
    [moveInto, announce, client, workspaceId, move],
  )

  return (
    <section className="view" aria-labelledby="board-heading">
      <ScopeBar>
        <CreateTask projectId={search.project} />
      </ScopeBar>
      <div className="view__bar view__bar--sub">
        <h1 id="board-heading" className="view__title">
          Board
        </h1>
      </div>

      {canMove ? null : (
        <div className="board__notice">
          <GapNotice what="Cards cannot be moved yet." tracker="D-061">
            <span>{unavailable}</span>
          </GapNotice>
        </div>
      )}
      {move.isError ? (
        <div className="board__notice">
          <ErrorNotice error={move.error} />
        </div>
      ) : null}

      <DndContext
        sensors={sensors}
        collisionDetection={closestCenter}
        onDragEnd={onDragEnd}
        accessibility={{ announcements }}
      >
        <div className="board">
          {TASK_STATES.map((state) => (
            <BoardColumn
              key={state}
              workspaceId={workspaceId}
              state={state}
              projectId={search.project}
              q={search.q}
              onOpen={openTask}
              draggable={canMove}
            />
          ))}
        </div>
      </DndContext>

      {search.task === undefined ? null : <TaskDrawer taskId={search.task} />}
    </section>
  )
}

/** The task behind a dragged card, from whichever column's page holds it. */
function findCachedTask(
  client: ReturnType<typeof useQueryClient>,
  workspaceId: string,
  taskId: string,
): Task | undefined {
  for (const query of client.getQueryCache().findAll({ queryKey: keys.taskLists(workspaceId) })) {
    const data = query.state.data as { pages?: { data: Task[] }[] } | undefined
    for (const page of data?.pages ?? []) {
      const found = page.data.find((row) => row.id === taskId)
      if (found !== undefined) return found
    }
  }
  return undefined
}
