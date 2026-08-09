/**
 * Stand-in for the real client (docs/05-API-SPEC.md). The floor harness must not
 * add a fetch/OpenAPI layer, because that layer is product code and would make
 * the measured floor larger than the dependency floor it exists to isolate.
 *
 * It lives under `floor/fixtures/` rather than beside the product client for
 * that same reason. It used to be `src/api.ts` and the ladder imported the real
 * route components; once C-018 put product code at those paths, every rung of
 * the ladder would have measured product bytes and BUNDLE-FLOOR.md would have
 * stopped meaning what its title says.
 */
export interface Task {
  readonly id: string
  readonly key: string
  readonly title: string
  readonly status: 'open' | 'in_progress' | 'done'
}

const STATUSES = ['open', 'in_progress', 'done'] as const

export function fetchTasks(count: number): Promise<Task[]> {
  const tasks: Task[] = Array.from({ length: count }, (_unused, i) => ({
    id: String(i),
    key: `TF-${i + 1}`,
    title: `Task row ${i + 1}`,
    status: STATUSES[i % STATUSES.length] ?? 'open',
  }))
  return Promise.resolve(tasks)
}
