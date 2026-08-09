/**
 * What is on which environment (`docs/45` §The two clocks).
 *
 * # The surface the product could not produce
 *
 * "Which feature is on staging" is the question a release conversation is made
 * of, and no status board can answer it: a status says what state the work is
 * in, an environment says where it has *reached*, and merging them into columns
 * like `In QA` / `On Staging` spends the second fact describing the first.
 *
 * So this is a board whose columns are environments, in deployment order, and
 * whose cards are the same cards the status board draws. A task appears under
 * the environment it is on now; its history of getting there is on the task.
 *
 * # Why it is a selection surface and not a drag one
 *
 * A card does not move here by being dragged. An environment changes because
 * something was *deployed* — the promotion is a record of a real event, not a
 * wish. But a deploy carries many tasks at once, so the surface that reports
 * where things are is also the natural place to say what went together: tick
 * the ones that shipped, name the release, and every one of them moves
 * (`docs/45`). A drag would move them one at a time and record eleven separate
 * events for a single deployment.
 *
 * # Why "not yet deployed" is a column
 *
 * A project's work is not all in the pipeline, and the tasks that have reached
 * nothing are exactly the ones a release conversation is about to ask after.
 * Leaving them out would make the columns add up to less than the project and
 * quietly hide the answer.
 */
import { useMemo, useState, type ReactElement } from 'react'
import { Checkbox } from '@schnsrw/design-system'
import { useQueries, useQuery, useQueryClient } from '@tanstack/react-query'

import { listEnvironments } from '../api/environments'
import { keys } from '../api/keys'
import { listTasks, type Task } from '../api/tasks'
import { PageHeader } from '../shell/PageHeader'
import { EmptyState } from '../shell/EmptyState'
import { ErrorNotice } from '../shell/notice'
import { useAppSearch, useOpenTask } from '../shell/navigation'
import { useWorkspaceId } from '../shell/session'
import { TaskCard } from '../tasks/TaskCard'
import { filterFromSearch } from '../tasks/query'
import { ReleaseBar } from './ReleaseBar'
import { Releases } from './Releases'

/** One column's worth. Deliberately small: this is a survey, not a backlog. */
const PER_COLUMN = 50

export function EnvironmentView(): ReactElement {
  const workspaceId = useWorkspaceId()
  const search = useAppSearch()
  const openTask = useOpenTask()
  const project = search.project

  const client = useQueryClient()
  // Across columns on purpose: a release usually promotes what is sitting on
  // qa, but a hotfix that never went there belongs in the same batch.
  const [selected, setSelected] = useState<ReadonlySet<string>>(() => new Set())

  const environments = useQuery({
    queryKey: keys.environments(workspaceId, project ?? ''),
    queryFn: ({ signal }) => listEnvironments(workspaceId, project as string, signal),
    enabled: workspaceId !== '' && project !== undefined,
    staleTime: 5 * 60_000,
  })

  const ordered = useMemo(
    () => [...(environments.data?.data ?? [])].sort((a, b) => a.position - b.position),
    [environments.data],
  )

  // One query per column, which is how the status board works too: the filter
  // grammar already answers "tasks on environment X", and a bespoke grouped
  // endpoint would be a second query path to keep in step with it. A pipeline
  // has four or five stages, not fifty.
  const columns = useQueries({
    queries: [
      ...ordered.map((environment) => ({
        queryKey: keys.taskList(workspaceId, {
          filter: { ...filterFromSearch(search), environment: environment.id },
          limit: PER_COLUMN,
        }),
        queryFn: ({ signal }: { signal: AbortSignal }) =>
          listTasks(
            workspaceId,
            {
              filter: { ...filterFromSearch(search), environment: environment.id },
              limit: PER_COLUMN,
            },
            signal,
          ),
        enabled: workspaceId !== '' && project !== undefined,
      })),
      {
        // The empty value is how the grammar spells "unset" (`docs/27`).
        queryKey: keys.taskList(workspaceId, {
          filter: { ...filterFromSearch(search), environment: '' },
          limit: PER_COLUMN,
        }),
        queryFn: ({ signal }: { signal: AbortSignal }) =>
          listTasks(
            workspaceId,
            { filter: { ...filterFromSearch(search), environment: '' }, limit: PER_COLUMN },
            signal,
          ),
        enabled: workspaceId !== '' && project !== undefined,
      },
    ],
  })

  if (project === undefined) {
    return (
      <section className="view" aria-labelledby="page-title">
        <PageHeader title="Environments" />
        <EmptyState
          title="Choose a project"
          detail="Environments belong to a project, so this view needs one. Pick it in the sidebar."
        />
      </section>
    )
  }

  if (environments.error) {
    return (
      <section className="view" aria-labelledby="page-title">
        <PageHeader title="Environments" />
        <ErrorNotice error={environments.error} />
      </section>
    )
  }

  if (!environments.isPending && ordered.length === 0) {
    return (
      <section className="view" aria-labelledby="page-title">
        <PageHeader title="Environments" />
        <EmptyState
          title="No environments yet"
          detail="Add the stages this project deploys through — dev, qa, staging, production — and this becomes the answer to “what is on staging”."
        />
      </section>
    )
  }

  const lanes = [
    ...ordered.map((environment, index) => ({
      key: environment.id,
      name: environment.name,
      tasks: (columns[index]?.data?.data ?? []) as readonly Task[],
      pending: columns[index]?.isPending ?? true,
    })),
    {
      key: 'none',
      name: 'Not yet deployed',
      tasks: (columns[ordered.length]?.data?.data ?? []) as readonly Task[],
      pending: columns[ordered.length]?.isPending ?? true,
    },
  ]

  return (
    <section className="view view--pipeline" aria-labelledby="page-title">
      <PageHeader title="Environments" />
      <div className="board" role="list" aria-label="Environments">
        {lanes.map((lane) => (
          <section className="column" key={lane.key} role="listitem" aria-label={lane.name}>
            <header className="column__head">
              <h2 className="column__title">{lane.name}</h2>
              <span className="column__count">{lane.pending ? '…' : lane.tasks.length}</span>
            </header>
            <div className="column__body">
              {lane.tasks.length === 0 && !lane.pending ? (
                <p className="column__empty">
                  {lane.key === 'none' ? 'Everything has been deployed.' : 'Nothing here.'}
                </p>
              ) : null}
              {lane.tasks.map((task) => (
                <div className="lane__pick" key={task.id}>
                  <Checkbox
                    checked={selected.has(task.id)}
                    aria-label={`Include ${task.key} in a release`}
                    onChange={() =>
                      setSelected((current) => {
                        const next = new Set(current)
                        if (!next.delete(task.id)) next.add(task.id)
                        return next
                      })
                    }
                  />
                  <TaskCard task={task} onOpen={openTask} />
                </div>
              ))}
            </div>
          </section>
        ))}
      </div>

      <ReleaseBar
        workspaceId={workspaceId}
        projectId={project}
        environments={ordered}
        selected={[...selected]}
        onClear={() => setSelected(new Set())}
        onCut={() => {
          // The columns are five separate queries keyed by environment, and a
          // release moves rows between them. Refetching the lot is the honest
          // answer: patching them by hand would mean re-deriving the server's
          // decision about which column each task now belongs in.
          void client.invalidateQueries({ queryKey: keys.taskLists(workspaceId) })
        }}
      />

      <Releases workspaceId={workspaceId} projectId={project} />
    </section>
  )
}

export default EnvironmentView
