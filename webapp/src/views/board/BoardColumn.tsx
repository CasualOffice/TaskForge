/**
 * One column of the board: its own keyset-paged query, virtualized.
 *
 * # The failure this module prevents
 *
 * Fetching the project and grouping it in the browser. That is the design
 * mistake docs/42 §Rendering strategy names by hand — "the client never
 * downloads a project to filter it locally... makes a tracker feel fine in
 * development and unusable at a real customer". Each column therefore asks the
 * server for *its* column, ordered by board rank, and pages with a cursor like
 * everything else.
 *
 * # A column is a workflow status, not a permanent state
 *
 * The default workflow has six statuses across the five states of `docs/23`:
 * "In Progress" and "Blocked" are both `ACTIVE`. Grouping by state puts them in
 * one column and throws away the distinction the workflow's author created —
 * which is exactly the distinction a board exists to show. Columns therefore
 * filter on `status`, and fall back to `state` only where no workflow is
 * readable at all.
 *
 * # Why each column virtualizes separately
 *
 * A column is an independent scroll container, so a shared virtualizer would
 * have to model six viewports. Six small ones is less code and holds the
 * "board with 500 cards, no jank" target the same way.
 */
import { useEffect, useMemo, useRef, type ReactElement } from 'react'
import { useDroppable } from '@dnd-kit/core'
import { useVirtualizer } from '@tanstack/react-virtual'

import type { TaskQuery } from '../../api/tasks'
import type { AppSearch } from '../../router'
import { ErrorNotice } from '../../shell/notice'
import { useTaskFeed } from '../../tasks/feed'
import { filterFromSearch } from '../../tasks/query'
import { BoardCard } from './BoardCard'

/** Roughly a card with a two-line title and a snippet; the virtualizer measures the rest. */
const CARD_HEIGHT = 132
const PREFETCH_MARGIN = 6

export interface Column {
  /** The droppable id: a status id, or a state name when there is no workflow. */
  readonly id: string
  readonly title: string
  /** Present when the column is a real workflow status. */
  readonly statusId: string | undefined
  /** The permanent state, for the fallback grouping and for the card's colour. */
  readonly state: string
}

export function BoardColumn({
  workspaceId,
  column,
  search,
  onOpen,
  draggable,
}: {
  workspaceId: string
  column: Column
  /** The address, so a column inherits every filter the toolbar set. */
  search: AppSearch
  onOpen: (id: string) => void
  draggable: boolean
}): ReactElement {
  const { setNodeRef, isOver } = useDroppable({ id: column.id })
  const scrollRef = useRef<HTMLDivElement>(null)

  const spec = useMemo<TaskQuery>(() => {
    const base = filterFromSearch(search)
    // The column's own constraint wins over the toolbar's status filter: a
    // column IS a status, and letting the toolbar override it would make every
    // column show the same rows.
    const scoped = column.statusId === undefined
      ? { ...base, status: undefined, state: column.state }
      : { ...base, status: column.statusId }
    return {
      filter: scoped,
      // The board rank (ADR-013), which is what a board is ordered by. Not
      // `updated_at`: a card that jumps to the top because someone edited its
      // description is a board that will not hold still.
      sort: { key: 'position', descending: false },
      limit: 100,
    }
  }, [search, column.statusId, column.state])

  const feed = useTaskFeed(workspaceId, spec)

  const virtualizer = useVirtualizer({
    count: feed.rows.length,
    getScrollElement: () => scrollRef.current,
    estimateSize: () => CARD_HEIGHT,
    overscan: 6,
    // Cards are not a fixed height — a description snippet and a due date each
    // add a line — so the virtualizer measures them. Measurements are cached by
    // KEY and not by index: after a card is dragged out of a column, index 2 is
    // a different task, and a cache keyed on the index would lay the new one out
    // at the old one's height and leave a visible hole in the column.
    getItemKey: (index) => feed.rows[index]?.id ?? index,
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
      aria-labelledby={`column-${column.id}`}
    >
      <header className="column__head">
        <span className={`column__dot column__dot--${column.state}`} aria-hidden="true" />
        <h2 id={`column-${column.id}`} className="column__title">
          {column.title}
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
                ref={virtualizer.measureElement}
                data-index={item.index}
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
