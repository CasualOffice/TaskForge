import { useState, type CSSProperties, type ReactElement } from 'react'
import {
  closestCenter,
  DndContext,
  KeyboardSensor,
  PointerSensor,
  useSensor,
  useSensors,
  type DragEndEvent,
} from '@dnd-kit/core'
import {
  arrayMove,
  SortableContext,
  sortableKeyboardCoordinates,
  useSortable,
  verticalListSortingStrategy,
} from '@dnd-kit/sortable'

const INITIAL = ['TF-1', 'TF-2', 'TF-3', 'TF-4', 'TF-5']

function Card({ id }: { id: string }): ReactElement {
  const { attributes, listeners, setNodeRef, transform, transition } = useSortable({ id })
  // Written out rather than pulled from @dnd-kit/utilities: docs/42 commits to an
  // exact dependency set and the floor must measure that set, not a superset.
  const style: CSSProperties = {
    transform: transform === null ? undefined : `translate3d(${transform.x}px, ${transform.y}px, 0)`,
    transition: transition ?? undefined,
    padding: 8,
    border: '1px solid #ccc',
    marginBottom: 4,
  }
  return (
    <div ref={setNodeRef} style={style} {...attributes} {...listeners}>
      {id}
    </div>
  )
}

/**
 * docs/42 §Accessibility makes the keyboard sensor a requirement, not an option,
 * so the floor includes it deliberately — a pointer-only board would measure
 * smaller than anything this product is allowed to ship.
 */
export function Board(): ReactElement {
  const [items, setItems] = useState<string[]>(INITIAL)
  const sensors = useSensors(
    useSensor(PointerSensor),
    useSensor(KeyboardSensor, { coordinateGetter: sortableKeyboardCoordinates }),
  )

  function onDragEnd(event: DragEndEvent): void {
    const { active, over } = event
    if (over === null || active.id === over.id) return
    setItems((prev) => {
      const from = prev.indexOf(String(active.id))
      const to = prev.indexOf(String(over.id))
      return from < 0 || to < 0 ? prev : arrayMove(prev, from, to)
    })
  }

  return (
    <DndContext sensors={sensors} collisionDetection={closestCenter} onDragEnd={onDragEnd}>
      <SortableContext items={items} strategy={verticalListSortingStrategy}>
        {items.map((id) => (
          <Card key={id} id={id} />
        ))}
      </SortableContext>
    </DndContext>
  )
}
