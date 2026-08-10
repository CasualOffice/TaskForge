/**
 * Reports — a filter plus an aggregation (`docs/38`, ADR-027).
 *
 * # No new noun
 *
 * `docs/01`'s simplicity contract: reporting adds no user-facing noun. A report
 * is the filter the list already speaks, plus a dimension to slice it by. The
 * body carries the same `field: value` map an address bar does, so a report and
 * the list it came from cannot disagree about what "open bugs" means.
 *
 * # Two people running the same report see different numbers
 *
 * The permission filter is injected per viewer, so a manager's total and a
 * guest's are both right. That is why the answer carries the scope it was
 * computed over, and why anything rendering it should say so.
 */
import { request } from './http'
import type { TaskFilter } from './tasks'

/** The closed dimension set (`docs/38`). */
export const DIMENSIONS = [
  'status',
  'state',
  'type',
  'priority',
  'project',
  'team',
  'environment',
  'reporter',
  'milestone',
  'assignee',
] as const

export type Dimension = (typeof DIMENSIONS)[number]

export interface ReportGroup {
  /** `null` is a real slice — unassigned, untriaged, on no environment. */
  readonly key: string | null
  readonly bucket_start?: string
  readonly total: number
}

/** What this build maintains a bounded query for (`docs/38` §Measures). */
export const MEASURES = [
  { key: 'count', label: 'How many', unit: 'tasks' },
  { key: 'cycle_time', label: 'Cycle time (median)', unit: 'seconds' },
  { key: 'p90_cycle_time', label: 'Cycle time (90th percentile)', unit: 'seconds' },
  { key: 'lead_time', label: 'Lead time (median)', unit: 'seconds' },
  { key: 'throughput', label: 'Throughput', unit: 'tasks' },
] as const

export type MeasureKey = (typeof MEASURES)[number]['key']

export interface ReportResult {
  readonly group_by: string
  readonly measure: string
  /** `seconds` for a duration, `tasks` for a count — never guessed. */
  readonly unit: string
  readonly groups: readonly ReportGroup[]
  readonly total: number
  /** How many projects the numbers were computed over. */
  readonly scope: { readonly projects: number }
}

export function runReport(
  workspaceId: string,
  input: { filter?: TaskFilter; groupBy: Dimension; measure?: MeasureKey; limit?: number },
  signal?: AbortSignal,
): Promise<ReportResult> {
  return request<ReportResult>('/api/v1/reports/run', {
    method: 'POST',
    workspaceId,
    signal,
    body: {
      group_by: input.groupBy,
      ...(input.measure === undefined ? {} : { measure: input.measure }),
      ...(input.filter === undefined ? {} : { filter: input.filter }),
      ...(input.limit === undefined ? {} : { limit: input.limit }),
    },
  })
}
