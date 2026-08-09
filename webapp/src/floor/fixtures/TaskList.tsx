import { useRef, type ReactElement } from 'react'
import { useQuery } from '@tanstack/react-query'
import { useVirtualizer } from '@tanstack/react-virtual'

import { fetchTasks, type Task } from './tasks'

/**
 * Exercises TanStack Query and TanStack Virtual together, which is how the real
 * list route works (docs/42 §Rendering strategy: virtualize everything
 * unbounded). Both libraries must be genuinely reachable at runtime or the
 * measurement is of a tree-shaken shell rather than of the shell.
 */
export function TaskList(): ReactElement {
  const parentRef = useRef<HTMLDivElement>(null)

  const { data, isPending } = useQuery<Task[]>({
    queryKey: ['tasks', { limit: 2000 }],
    queryFn: () => fetchTasks(2000),
  })

  const rows = data ?? []
  const virtualizer = useVirtualizer({
    count: rows.length,
    getScrollElement: () => parentRef.current,
    estimateSize: () => 36,
    overscan: 12,
  })

  if (isPending) return <p>Loading tasks…</p>

  return (
    <div ref={parentRef} style={{ height: 420, overflowY: 'auto', border: '1px solid #ccc' }}>
      <div style={{ height: virtualizer.getTotalSize(), position: 'relative' }}>
        {virtualizer.getVirtualItems().map((item) => {
          const task = rows[item.index]
          if (task === undefined) return null
          return (
            <div
              key={task.id}
              style={{
                position: 'absolute',
                top: 0,
                left: 0,
                width: '100%',
                height: item.size,
                transform: `translateY(${item.start}px)`,
              }}
            >
              <span>{task.key}</span> <span>{task.title}</span> <span>{task.status}</span>
            </div>
          )
        })}
      </div>
    </div>
  )
}
