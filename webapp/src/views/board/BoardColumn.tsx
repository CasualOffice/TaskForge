/**
 * One column of the board: its own keyset-paged query, virtualized.
 *
 * # The failure this module prevents
 *
 * Fetching the project and grouping it in the browser. That is the design
 * mistake docs/42 §Rendering strategy names by hand — "the client never
 * downloads a project to filter it locally... makes a tracker feel fine in
 * development and unusable at a real customer". Each column therefore asks the
 * server for *its* state, ordered by board rank, and pages with a cursor like
 * everything else.
 *
 * # Why each column virtualizes separately
 *
 * A column is an independent scroll container, so a shared virtualizer would
 * have to model five viewports. Five small ones is less code and holds the
 * "board with 500 cards, no jank" target the same way.
 */
import { useEffect, useMemo, useRef, type ReactElement } from 'react'
import { useDroppable } from '@dnd-kit/core'
import { useVirtualizer } from '@tanstack/react-virtual'

import type { TaskQuery, TaskState } from '../../api/tasks'
import { ErrorNotice } from '../../shell/notice'
import { useTaskFeed } from '../../tasks/feed'
import { stateLabel } from '../../tasks/present'
import { BoardCard } from './BoardCard'

const CARD_HEIGHT = 96
const PREFETCH_MARGIN = 6

export function BoardColumn({
  workspaceId,
  state,
  projectId,
  q,
  onOpen,
  draggable,
}: {
  workspaceId: string
  state: TaskState
  projectId: string | undefined
  q: string | undefined
  onOpen: (id: string) => void
  draggable: boolean
}): ReactElement {
  const { setNodeRef, isOver } = useDroppable({ id: state })
  const scrollRef = useRef<HTMLDivElement>(null)

  const spec = useMemo<TaskQuery>(
    () => ({
      filter: {
        state,
        ...(projectId === undefined ? {} : { project: projectId }),
        ...(q === undefined ? {} : { q }),
      },
      // The board rank (ADR-013), which is what a board is ordered by. Not
      // `updated_at`: a card that jumps to the top because someone edited its
      // description is a board that will not hold still.
      sort: { key: 'position', descending: false },
      limit: 100,
    }),
    [state, projectId, q],
  )

  const feed = useTaskFeed(workspaceId, spec)

  const virtualizer = useVirtualizer({
    count: feed.rows.length,
    getScrollElement: () => scrollRef.current,
    estimateSize: () => CARD_HEIGHT,
    overscan: 6,
  })

  const items = virtualizer.getVirtualItems()
  const lastVisible = items.at(-1)?.index ?? 0

  useEffect(() => {
    if (feed.hasMore && lastVisible >= feed.rows.length - PREFETCH_MARGIN) feed.fetchMore()
  }, [feed, lastVisible])

  return (
    <section
      className={`column${isOver ? ' column--over' : ''}`}
      ref={setNodeRef}
      aria-labelledby={`column-${state}`}
    >
      <header className="column__head">
        <h2 id={`column-${state}`} className="column__title">
          {stateLabel(state)}
        </h2>
        <span className="column__count">
          {feed.rows.length}
          {feed.hasMore ? '+' : ''}
        </span>
      </header>

      <div className="column__body" ref={scrollRef}>
        {feed.error != null ? <ErrorNotice error={feed.error} /> : null}
        {feed.isPending ? <p className="field__hint">Loading…</p> : null}
        {!feed.isPending && feed.rows.length === 0 && feed.error == null ? (
          <p className="field__hint column__empty">Nothing here.</p>
        ) : null}

        <div style={{ height: virtualizer.getTotalSize(), position: 'relative' }}>
          {items.map((item) => {
            const task = feed.rows[item.index]
            if (task === undefined) return null
            return (
              <div
                key={task.id}
                className="column__slot"
                style={{
                  position: 'absolute',
                  top: 0,
                  left: 0,
                  width: '100%',
                  transform: `translateY(${item.start}px)`,
                }}
              >
                <BoardCard task={task} onOpen={onOpen} draggable={draggable} />
              </div>
            )
          })}
        </div>
      </div>
    </section>
  )
}
