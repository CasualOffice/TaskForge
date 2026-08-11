/**
 * Reading a group key.
 *
 * # Why this is shared rather than copied
 *
 * A report answers in the database's vocabulary — `019fe76c-…` for an assignee,
 * `BACKLOG` for a state, `null` for "no value". Turning that into something a
 * person can read takes three lookups and one judgement call, and the judgement
 * call is the reason this is a module rather than a helper on each page: `null`
 * grouped by team means **Untriaged**, `null` grouped by assignee means
 * **Unassigned**, and those are different facts about the work. A dashboard and
 * a report that disagreed about which word to use would be two products.
 *
 * # Why the dimension decides, not the value
 *
 * `null` is a real slice, not a missing one. Rendering it as an empty cell —
 * or dropping it — hides the single most actionable number on a workload
 * dashboard, which is how much work nobody has picked up.
 */
import { listProjects } from '../../api/projects'
import { listTeams } from '../../api/admin'
import { listMembers } from '../../api/workspaces'
import { keys } from '../../api/keys'
import type { Dimension } from '../../api/reports'
import { priorityLabel, stateLabel, typeLabel } from '../../tasks/present'
import { useQuery } from '@tanstack/react-query'

/** What "no value" is called, which depends on the question. */
export const EMPTY_LABEL: Partial<Record<Dimension, string>> = {
  team: 'Untriaged',
  assignee: 'Unassigned',
  reporter: 'No reporter',
  project: 'No project',
  milestone: 'No milestone',
  environment: 'No environment',
}

/**
 * The dimensions whose keys are ids, and therefore need a name fetched.
 *
 * `status`, `milestone` and `environment` are ids too and are deliberately
 * absent: their names live in a *project's* configuration, so at workspace
 * scope two projects can hold two different statuses with the same name and
 * there is no one vocabulary to read them through. A slice nobody can read is
 * not a slice, which is why nothing here offers them.
 */
const NEEDS_NAMES: readonly Dimension[] = ['project', 'team', 'assignee', 'reporter']

export function needsNames(dimension: Dimension): boolean {
  return NEEDS_NAMES.includes(dimension)
}

/**
 * Seconds, as a person would say them.
 *
 * A cycle time of 271 828 seconds is a number nobody can act on. The unit comes
 * from the server rather than being inferred from the measure name, so a client
 * that has not heard of a new measure still formats it correctly or plainly.
 */
export function duration(seconds: number): string {
  if (seconds < 90) return `${Math.round(seconds)}s`
  const minutes = seconds / 60
  if (minutes < 90) return `${Math.round(minutes)}m`
  const hours = minutes / 60
  if (hours < 48) return `${hours.toFixed(1)}h`
  return `${(hours / 24).toFixed(1)}d`
}

/** A measured value in the unit the *server* reported, never a guessed one. */
export function formatValue(total: number, unit: string): string {
  return unit === 'seconds' ? duration(total) : String(total)
}

/**
 * The name-resolving vocabularies.
 *
 * Fetched once per workspace and shared by every tile on a dashboard through
 * the query cache — twelve tiles grouped by assignee issue one member request
 * between them, not twelve. `staleTime` is generous because a workspace's
 * people and projects change on a human timescale, not a dashboard's.
 */
export function useVocabulary(workspaceId: string, dimensions: readonly Dimension[]): {
  label: (key: string | null, dimension: Dimension) => string
  isPending: boolean
} {
  const wanted = dimensions.filter(needsNames)
  const projects = useQuery({
    queryKey: keys.projects(workspaceId),
    queryFn: ({ signal }) => listProjects(workspaceId, signal),
    enabled: workspaceId !== '' && wanted.includes('project'),
    staleTime: 60_000,
  })
  const teams = useQuery({
    queryKey: keys.teams(workspaceId),
    queryFn: ({ signal }) => listTeams(workspaceId, signal),
    enabled: workspaceId !== '' && wanted.includes('team'),
    staleTime: 60_000,
  })
  const members = useQuery({
    queryKey: keys.members(workspaceId),
    queryFn: ({ signal }) => listMembers(workspaceId, signal),
    enabled: workspaceId !== '' && (wanted.includes('assignee') || wanted.includes('reporter')),
    staleTime: 60_000,
  })

  const names = new Map<string, string>()
  for (const project of projects.data?.data ?? []) names.set(project.id, project.name)
  for (const team of teams.data?.data ?? []) names.set(team.id, team.name)
  for (const member of members.data?.data ?? []) names.set(member.user_id, member.display_name)

  return {
    label: (key: string | null, dimension: Dimension): string => {
      if (key === null) return EMPTY_LABEL[dimension] ?? 'None'
      if (dimension === 'type') return typeLabel(key)
      if (dimension === 'priority') return priorityLabel(key)
      if (dimension === 'state') return stateLabel(key)
      // The id itself, when the name has not arrived yet or the person has left
      // the workspace. Ugly on purpose: it is a fact, and inventing "Unknown"
      // would make a departed member indistinguishable from a bug.
      return names.get(key) ?? key
    },
    isPending:
      (wanted.includes('project') && projects.isPending) ||
      (wanted.includes('team') && teams.isPending) ||
      ((wanted.includes('assignee') || wanted.includes('reporter')) && members.isPending),
  }
}
