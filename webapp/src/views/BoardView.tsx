/**
 * The board.
 *
 * # A column is a workflow status
 *
 * The default workflow has six statuses across the five permanent states of
 * `docs/23` — "In Progress" and "Blocked" are both `ACTIVE`. Grouping by state
 * collapses them and throws away the one distinction the workflow's author
 * created, which is the distinction a board exists to show. **D-061 is settled
 * this way**: columns are statuses, in the workflow's own `position` order, and
 * state survives only as the colour of a column's dot and as the fallback
 * grouping when no workflow can be read at all.
 *
 * # The board picks a project rather than asking for one
 *
 * A workflow belongs to a project, so a board with no project has no columns and
 * no legal moves. It used to render a banner explaining that — which is an
 * architecture lesson delivered to someone who wanted to drag a card. It now
 * selects the first project instead, and the toolbar's dropdown is how anyone
 * changes it. The cost, stated: there is no all-projects board. That is not a
 * limitation being hidden — a board across two workflows has no coherent set of
 * columns, and inventing one would be inventing a decision.
 *
 * # A drop is checked before it is sent
 *
 * `docs/23` gives each transition a `required_permission` on top of
 * `task.transition`, so an actor who may move tasks in general may still not
 * make one particular move. Refusing here means the card never appears to move
 * and then springs back, which reads as a bug rather than as a refusal.
 */
import { useCallback, useEffect, useMemo, useState, type ReactElement } from 'react'
import {
  DndContext,
  DragOverlay,
  KeyboardSensor,
  PointerSensor,
  closestCenter,
  useSensor,
  useSensors,
  type Announcements,
  type DragEndEvent,
} from '@dnd-kit/core'
import { useQuery, useQueryClient } from '@tanstack/react-query'

import { keys } from '../api/keys'
import { PERMISSIONS } from '../api/permissions'
import { listProjects } from '../api/projects'
import { TASK_STATES, type Task } from '../api/tasks'
import { useLiveUpdates } from '../live/useLiveUpdates'
import { TaskCard } from '../tasks/TaskCard'
import { useAnnounce } from '../shell/announce'
import { useAppSearch, useOpenTask, useUpdateSearch } from '../shell/navigation'
import { ErrorNotice } from '../shell/notice'
import { useAuthority } from '../shell/permissions'
import { useWorkspaceId } from '../shell/session'
import { useTaskTransition } from '../tasks/mutations'
import { stateLabel } from '../tasks/present'
import { useProjectWorkflow } from '../tasks/useWorkflow'
import { BoardColumn, type Column } from './board/BoardColumn'
import { CreateTask } from './CreateTask'
import { PageHeader } from '../shell/PageHeader'
import { WorkToolbar } from './filters/WorkToolbar'

export function BoardView(): ReactElement {
  const workspaceId = useWorkspaceId()
  const search = useAppSearch()
  const update = useUpdateSearch()
  const openTask = useOpenTask()
  const announce = useAnnounce()
  const client = useQueryClient()

  const projects = useQuery({
    queryKey: keys.projects(workspaceId),
    queryFn: ({ signal }) => listProjects(workspaceId, signal),
    enabled: workspaceId !== '',
    staleTime: 60_000,
  })

  const firstProject = projects.data?.data[0]?.id

  // Chosen, not demanded. `replace` so the back button does not have to step
  // through a redirect the user never asked for.
  useEffect(() => {
    if (search.project === undefined && firstProject !== undefined) {
      update({ project: firstProject })
    }
  }, [search.project, firstProject, update])

  const { workflow, moveTo } = useProjectWorkflow(search.project)
  const authority = useAuthority(search.project)
  const move = useTaskTransition(workspaceId)

  useLiveUpdates(workspaceId, search.project)

  const columns = useMemo<Column[]>(() => {
    if (workflow !== undefined) {
      return [...workflow.statuses]
        .sort((a, b) => a.position - b.position)
        .map((status) => ({
          id: status.id,
          title: status.name,
          statusId: status.id,
          state: status.state,
        }))
    }
    // No workflow readable — a workspace with no projects, or a server that does
    // not serve the route. The five permanent states are the only grouping that
    // is correct without one.
    return TASK_STATES.map((state) => ({
      id: state,
      title: stateLabel(state),
      statusId: undefined,
      state,
    }))
  }, [workflow])

  const canMove = workflow !== undefined && authority.can(PERMISSIONS.taskTransition)

  // Which card is in the air. Held here rather than in the card, because the
  // overlay is rendered by the context and has to know what to draw.
  const [dragged, setDragged] = useState<Task | undefined>(undefined)

  const sensors = useSensors(
    useSensor(PointerSensor, {
      // A few pixels of slop, so a click on a card's title is a click and not a
      // one-pixel drag that swallows it.
      activationConstraint: { distance: 4 },
    }),
    useSensor(KeyboardSensor),
  )

  const titleOf = useCallback(
    (id: string) => columns.find((column) => column.id === id)?.title ?? id,
    [columns],
  )

  /** dnd-kit speaks these to a live region; without them a keyboard drag is silent. */
  const announcements = useMemo<Announcements>(
    () => ({
      onDragStart: ({ active }) => `Picked up ${String(active.id)}.`,
      onDragOver: ({ over }) => (over === null ? 'No column.' : `Over ${titleOf(String(over.id))}.`),
      onDragEnd: ({ over }) =>
        over === null ? 'Move cancelled.' : `Dropped in ${titleOf(String(over.id))}.`,
      onDragCancel: () => 'Move cancelled.',
    }),
    [titleOf],
  )

  const onDragEnd = useCallback(
    (event: DragEndEvent) => {
      const { active, over } = event
      if (over === null) return
      const toStatusId = String(over.id)

      // The card being dragged is the one already in a list cache; reading it
      // from there rather than refetching keeps the optimistic update instant.
      const task = findCachedTask(client, workspaceId, String(active.id))
      if (task === undefined || task.status_id === toStatusId) return

      // The workflow decides, not the columns' adjacency. `docs/23`: a
      // transition exists or it does not, and a board that inferred one from two
      // columns sitting side by side would send moves the state machine refuses.
      const edge = moveTo(task.status_id, toStatusId)
      if (edge === undefined) {
        announce(`This workflow has no move from here into ${titleOf(toStatusId)}.`)
        return
      }
      if (!edge.permitted) {
        announce(`That move needs ${edge.needed ?? 'a permission you do not have'}.`)
        return
      }

      move.mutate(
        { task, toStatusId: edge.toStatusId, toState: edge.toState },
        { onSuccess: () => announce(`${task.key} moved to ${titleOf(toStatusId)}`) },
      )
    },
    [moveTo, announce, client, workspaceId, move, titleOf],
  )

  return (
    <section className="view" aria-labelledby="page-title">
      <PageHeader title="Board" actions={<CreateTask projectId={search.project} />} />
      {/* No sort control: a board is ordered by board rank (ADR-013) and by
          nothing else — offering "sort by due date" would silently disable the
          drag that writes that rank. */}
      <WorkToolbar />

      {move.isError ? (
        <div className="board__notice">
          <ErrorNotice error={move.error} />
        </div>
      ) : null}

      <DndContext
        sensors={sensors}
        collisionDetection={closestCenter}
        onDragStart={({ active }) =>
          setDragged((active.data.current as { task?: Task } | undefined)?.task)
        }
        onDragCancel={() => setDragged(undefined)}
        onDragEnd={(dragEnd) => {
          setDragged(undefined)
          onDragEnd(dragEnd)
        }}
        accessibility={{ announcements }}
      >
        <div className="board">
          {columns.map((column) => (
            <BoardColumn
              key={column.id}
              workspaceId={workspaceId}
              column={column}
              search={search}
              onOpen={openTask}
              draggable={canMove}
            />
          ))}
        </div>

        {/* The card being dragged, drawn once at the top of the page instead of
            moved inside the column it came from.
            
            A translated card stayed a child of `.column__body`, which is a
            scrolling region — so it was *clipped* by its own column and painted
            under every column to its right. No `z-index` can fix that: an
            element cannot escape an ancestor's `overflow` clip, whatever it is
            stacked at. `DragOverlay` renders outside that subtree, which is the
            only thing that can follow the pointer across columns. */}
        <DragOverlay dropAnimation={null}>
          {dragged === undefined ? null : (
            <div className="card__dragging">
              <TaskCard task={dragged} onOpen={() => {}} />
            </div>
          )}
        </DragOverlay>
      </DndContext>

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
