/**
 * Home: whose turn is it.
 *
 * # The job this answers, and the one it used to answer
 *
 * `docs/44` ranks "what is mine, and what changed while I was away" as the
 * highest-frequency job in the product — every session begins with it, and its
 * budget is about fifteen seconds. The surface that served it was a *filtered
 * task list*, which answers a different question: "which items match a query".
 * Those produce different orderings, and the gap between them is why the first
 * screen of the day was never useful.
 *
 * # Four courts, from the server
 *
 * `docs/45` defines a task's court from its owning team, its assignees and its
 * verification history. The server computes it (`GET /me/queue`) — composing it
 * from four filtered lists here would put a domain rule in TypeScript, which is
 * the mistake `docs/42` warns about for authority and which applies identically
 * here.
 *
 * # Why every court is shown to everyone
 *
 * There is no "role" field to switch on — a role is a set of grants, and the
 * same person is a developer on one project and a reviewer on another. So the
 * surface shows the courts that *have something in them*: a developer's day
 * fills "in your court", QA's fills "waiting to be verified", a lead's fills
 * triage. The shape of the screen follows what is true rather than what a
 * profile claims, and an empty court simply is not drawn.
 */
import type { ReactElement } from 'react'
import { useQuery } from '@tanstack/react-query'

import { readQueue } from '../api/custody'
import { keys } from '../api/keys'
import type { Task } from '../api/tasks'
import { EmptyState } from '../shell/EmptyState'
import { PageHeader } from '../shell/PageHeader'
import { ErrorNotice } from '../shell/notice'
import { useOpenTask } from '../shell/navigation'
import { useWorkspaceId } from '../shell/session'
import { SkeletonRows } from '../shell/Skeleton'
import { TaskCard } from '../tasks/TaskCard'

export function HomeView(): ReactElement {
  const workspaceId = useWorkspaceId()
  const openTask = useOpenTask()

  const queue = useQuery({
    queryKey: keys.queue(workspaceId),
    queryFn: ({ signal }) => readQueue(workspaceId, signal),
    enabled: workspaceId !== '',
  })

  const data = queue.data
  const courts = [
    {
      key: 'mine',
      title: 'In your court',
      why: 'Assigned to you and still open.',
      tasks: data?.mine ?? [],
    },
    {
      key: 'verify',
      title: 'Waiting to be verified',
      why: 'Pushed to an environment and not passed there since.',
      tasks: data?.awaiting_verification ?? [],
    },
    {
      key: 'team',
      title: 'Your teams’ queue',
      why: 'Owned by a team you are in, and nobody has picked it up.',
      tasks: data?.team_queue ?? [],
    },
    {
      key: 'triage',
      title: 'Needs triage',
      why: 'Open and owned by no team yet.',
      tasks: data?.triage ?? [],
    },
  ].filter((court) => court.tasks.length > 0)

  const total = courts.reduce((sum, court) => sum + court.tasks.length, 0)

  return (
    <section className="view" aria-labelledby="page-title">
      <PageHeader
        title="Home"
        count={queue.isPending ? undefined : `${total} waiting on someone`}
      />

      <div className="view__body">
        {queue.error ? <ErrorNotice error={queue.error} /> : null}
        {queue.isPending ? <SkeletonRows rows={5} height={64} label="Loading your work" /> : null}

        {/* Nothing in any court is a real answer and a good one, so it is said
            plainly rather than left as four empty headings. */}
        {!queue.isPending && courts.length === 0 && queue.error == null ? (
          <EmptyState
            title="Nothing is waiting on anyone"
            detail="No open work is assigned, queued, untriaged or awaiting verification in this workspace."
          />
        ) : null}

        {courts.map((court) => (
          <section className="court" key={court.key} aria-labelledby={`court-${court.key}`}>
            <header className="court__head">
              <h2 className="court__title" id={`court-${court.key}`}>
                {court.title}
              </h2>
              <span className="court__count">{court.tasks.length}</span>
              {/* The sentence is the point: a heading alone says what the list
                  is called, not why these tasks are in it. */}
              <span className="court__why">{court.why}</span>
            </header>
            <div className="court__cards">
              {court.tasks.map((task: Task) => (
                <TaskCard key={task.id} task={task} onOpen={openTask} />
              ))}
            </div>
          </section>
        ))}
      </div>
    </section>
  )
}

export default HomeView
