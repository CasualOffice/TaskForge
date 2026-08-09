/**
 * Every cache key in the app, in one table.
 *
 * # The failure this module prevents
 *
 * An invalidation that misses. TanStack Query matches keys by prefix, so a
 * mutation invalidating `['tasks']` clears every list — but only if every list
 * actually *starts* with `'tasks'`. Keys typed at call sites drift within a
 * week: one view uses `['task-list', …]`, its optimistic update invalidates
 * `['tasks', …]`, and the stale row survives a write with no error anywhere.
 *
 * # Every key starts with the workspace
 *
 * `docs/32` is about the server, but the cache is a client-side copy of tenant
 * data and switching workspaces must not show the previous tenant's rows for a
 * frame. Putting the workspace id first makes that structural: a workspace
 * switch invalidates by prefix, and no key can be written that omits it.
 */
import type { TaskQuery } from './tasks'

export const keys = {
  /** The session. Outside every workspace — a person spans them. */
  session: () => ['session'] as const,

  workspaces: () => ['workspaces'] as const,

  /** Everything under one tenant. The prefix a workspace switch clears. */
  workspace: (workspaceId: string) => ['ws', workspaceId] as const,

  members: (workspaceId: string) => ['ws', workspaceId, 'members'] as const,

  projects: (workspaceId: string) => ['ws', workspaceId, 'projects'] as const,

  workflow: (workspaceId: string, workflowId: string) =>
    ['ws', workspaceId, 'workflow', workflowId] as const,

  /** Every task list. The prefix any task mutation invalidates. */
  taskLists: (workspaceId: string) => ['ws', workspaceId, 'tasks'] as const,

  /**
   * One list. The spec is part of the key because two views with different
   * filters are different results, not the same result rendered twice.
   */
  taskList: (workspaceId: string, spec: TaskQuery) =>
    ['ws', workspaceId, 'tasks', 'list', spec] as const,

  task: (workspaceId: string, taskId: string) =>
    ['ws', workspaceId, 'tasks', 'one', taskId] as const,

  comments: (workspaceId: string, taskId: string) =>
    ['ws', workspaceId, 'tasks', 'one', taskId, 'comments'] as const,
} as const
