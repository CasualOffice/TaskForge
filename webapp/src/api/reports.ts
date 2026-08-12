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
  // The *oldest*, not the median: "how old is the work" is asked as "what has
  // been sitting longest", and a median hides the one task the question is
  // about. `docs/38` defines age over open work only, which the server enforces
  // rather than trusting the filter to.
  { key: 'age', label: 'Age of oldest open', unit: 'seconds' },
  { key: 'p50_age', label: 'Age of open work (median)', unit: 'seconds' },
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

/**
 * The time grain of a series (`docs/38` §The report model).
 *
 * Closed on both axes, because both are compiled into SQL: the field is one of
 * the datetime columns the report compiler knows how to `date_trunc`, and the
 * interval is one of the four grains `docs/38` budgets 400 buckets for.
 */
export const INTERVALS = ['day', 'week', 'month'] as const
export type Interval = (typeof INTERVALS)[number]

/**
 * Exactly what `bucket_of` accepts, and nothing more.
 *
 * `completed_at` is the one a throughput trend reads like it wants, and it is
 * deliberately absent: there is no such column — completion is a state entry,
 * which is why `throughput` is its own measure rather than a bucket field. A
 * client type wide enough to express it would be a type that compiles into a
 * `400`.
 */
export const BUCKET_FIELDS = ['created_at', 'updated_at', 'due_at'] as const
export type BucketField = (typeof BUCKET_FIELDS)[number]

export interface Bucket {
  readonly field: BucketField
  readonly interval: Interval
}

export interface ReportInput {
  readonly filter?: TaskFilter
  readonly groupBy: Dimension
  readonly measure?: MeasureKey
  /** Present only for a series; absent for a single slice. */
  readonly bucket?: Bucket
  readonly limit?: number
}

export function runReport(
  workspaceId: string,
  input: ReportInput,
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
      ...(input.bucket === undefined ? {} : { bucket: input.bucket }),
      ...(input.limit === undefined ? {} : { limit: input.limit }),
    },
  })
}
