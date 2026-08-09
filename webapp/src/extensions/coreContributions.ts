/**
 * The extension point registry, as the client sees it.
 *
 * # Why the drawer's panels come from here and not from a JSX list
 *
 * `docs/34` and C-017: the core's own panels render **through the registry**, so
 * the contract is exercised in Phase 1 rather than discovered to be wrong in
 * Phase 3 when third parties depend on it. A drawer that hard-codes its sections
 * proves nothing about the seam, and the first plugin to contribute a panel
 * would find the drawer has no place to put one.
 *
 * So the drawer renders `contributions(UiTaskPanel)` in order, and a panel this
 * client has no component for is rendered as a declared-but-unimplemented slot
 * rather than skipped — the registry says the point exists, and silently
 * dropping it would make the registry a decoration.
 *
 * # This table is a MIRROR, and that is a known cost
 *
 * The authority is `crates/casual-task-plugin-contract/src/core_contributions.rs`.
 * It is a Rust table with **no HTTP surface**: nothing serves the registry, so a
 * browser cannot read it. The rows below are copied from that file — same
 * points, same slugs, same titles, same order — and they will drift the day
 * someone edits one side only.
 *
 * The fix is an endpoint (`GET /api/v1/extensions` or the manifest in
 * `docs/34`), which is C-017's missing half. Until it exists, this comment is
 * the only thing holding the two tables together, and that is stated plainly
 * rather than left for the next reader to discover.
 */

/** The frontend points from `ExtensionPoint`. Backend points are not the client's. */
export type FrontendPoint =
  | 'ui.task.panel'
  | 'ui.task.badge'
  | 'ui.project.tab'
  | 'ui.command'
  | 'ui.settings.section'

export interface Contribution {
  readonly point: FrontendPoint
  /** Stable identifier. What a component keys its implementation off. */
  readonly slug: string
  /** Human title. What the registry says it is called. */
  readonly title: string
  /** `core` here; a plugin's id once plugins exist. `Provider::Core` is a label, not a tier. */
  readonly provider: 'core'
}

/** Copied from `CORE` in `core_contributions.rs`, in its declared order. */
const CORE: readonly Contribution[] = [
  // The task drawer, in the order it renders (docs/42 §Task drawer).
  { point: 'ui.task.panel', slug: 'details', title: 'Details', provider: 'core' },
  { point: 'ui.task.panel', slug: 'comments', title: 'Comments', provider: 'core' },
  { point: 'ui.task.panel', slug: 'attachments', title: 'Attachments', provider: 'core' },
  { point: 'ui.task.panel', slug: 'relations', title: 'Relations', provider: 'core' },
  { point: 'ui.task.panel', slug: 'activity', title: 'Activity', provider: 'core' },
  // Declared once it was built, not before: the registry says what exists, and
  // `unbuilt.ts` reads it to name what does not.
  { point: 'ui.task.panel', slug: 'subtasks', title: 'Subtasks', provider: 'core' },
  // Badges on cards and list rows, rendered from data already fetched.
  { point: 'ui.task.badge', slug: 'status', title: 'Status', provider: 'core' },
  { point: 'ui.task.badge', slug: 'priority', title: 'Priority', provider: 'core' },
  { point: 'ui.task.badge', slug: 'assignee', title: 'Assignee', provider: 'core' },
  { point: 'ui.task.badge', slug: 'due-date', title: 'Due date', provider: 'core' },
  // Project tabs.
  { point: 'ui.project.tab', slug: 'board', title: 'Board', provider: 'core' },
  { point: 'ui.project.tab', slug: 'list', title: 'List', provider: 'core' },
  { point: 'ui.project.tab', slug: 'reports', title: 'Reports', provider: 'core' },
  // The palette.
  { point: 'ui.command', slug: 'create-task', title: 'Create task', provider: 'core' },
  { point: 'ui.command', slug: 'go-to-project', title: 'Go to project', provider: 'core' },
  { point: 'ui.command', slug: 'assign', title: 'Assign', provider: 'core' },
  { point: 'ui.command', slug: 'transition', title: 'Change status', provider: 'core' },
  { point: 'ui.command', slug: 'search', title: 'Search', provider: 'core' },
  // Admin settings.
  { point: 'ui.settings.section', slug: 'members', title: 'Members', provider: 'core' },
  { point: 'ui.settings.section', slug: 'teams', title: 'Teams', provider: 'core' },
  { point: 'ui.settings.section', slug: 'roles', title: 'Roles', provider: 'core' },
  { point: 'ui.settings.section', slug: 'workflow', title: 'Workflow', provider: 'core' },
  { point: 'ui.settings.section', slug: 'extensions', title: 'Extensions', provider: 'core' },
]

/** Everything contributed at a point, in registration order. */
export function contributions(point: FrontendPoint): readonly Contribution[] {
  return CORE.filter((entry) => entry.point === point)
}

/** One contribution's declared title, for a slot that has no implementation yet. */
export function titleOf(point: FrontendPoint, slug: string): string | undefined {
  return CORE.find((entry) => entry.point === point && entry.slug === slug)?.title
}
