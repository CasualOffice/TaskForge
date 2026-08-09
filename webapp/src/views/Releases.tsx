/**
 * What has gone out, most recent first (`docs/45` §The two clocks).
 *
 * # Why this sits under the pipeline and not on its own page
 *
 * "What is on staging" and "what went out on Tuesday" are the same
 * conversation, asked a minute apart. Splitting them across two routes would
 * make the second one a place you have to remember exists — and a release
 * history nobody opens is a record that might as well not be kept.
 *
 * # Why the contents are fetched on expand
 *
 * A release carries up to two hundred tasks and a project accumulates releases
 * forever. Loading every task of every release to render a list of names would
 * spend the page's whole budget on rows nobody asked to see. The name, the
 * count and the date answer the question most of the time; the tasks are one
 * click away for the time it does not.
 */
import { useState, type ReactElement } from 'react'
import { Button } from '@schnsrw/design-system'
import { useQuery } from '@tanstack/react-query'

import { keys } from '../api/keys'
import { listReleases, readRelease, type Release } from '../api/releases'
import { EmptyState } from '../shell/EmptyState'
import { ErrorNotice } from '../shell/notice'
import { FLUSH } from '../shell/controls'
import { useOpenTask } from '../shell/navigation'
import { SkeletonRows } from '../shell/Skeleton'
import { formatRelative } from '../tasks/present'

export function Releases({
  workspaceId,
  projectId,
}: {
  workspaceId: string
  projectId: string
}): ReactElement {
  const releases = useQuery({
    queryKey: keys.releases(workspaceId, projectId),
    queryFn: ({ signal }) => listReleases(workspaceId, projectId, signal),
    enabled: workspaceId !== '' && projectId !== '',
  })

  return (
    <section className="rel2" aria-labelledby="releases-heading">
      <h2 className="rel2__heading" id="releases-heading">
        Releases
      </h2>

      {releases.error ? <ErrorNotice error={releases.error} /> : null}
      {releases.isPending ? <SkeletonRows rows={2} height={40} label="Loading releases" /> : null}

      {!releases.isPending && (releases.data?.data ?? []).length === 0 && !releases.error ? (
        <EmptyState
          title="Nothing has been released yet"
          detail="Select tasks on the pipeline above and record what went out together. After that, this is the answer to “what shipped on Tuesday”."
        />
      ) : null}

      <ul className="rel2__list">
        {(releases.data?.data ?? []).map((release) => (
          <ReleaseRow key={release.id} workspaceId={workspaceId} release={release} />
        ))}
      </ul>
    </section>
  )
}

function ReleaseRow({
  workspaceId,
  release,
}: {
  workspaceId: string
  release: Release
}): ReactElement {
  const [open, setOpen] = useState(false)
  const openTask = useOpenTask()

  const contents = useQuery({
    queryKey: keys.release(workspaceId, release.id),
    queryFn: ({ signal }) => readRelease(workspaceId, release.id, signal),
    // Only when asked. See the module note on why this is not eager.
    enabled: open,
  })

  return (
    <li className="rel2__item">
      <div className="rel2__row">
        <Button variant="subtle" style={FLUSH} aria-expanded={open} onClick={() => setOpen(!open)}>
          {release.name}
        </Button>
        <span className="rel2__when">{formatRelative(release.created_at)}</span>
        {release.note === null ? null : <span className="rel2__note">{release.note}</span>}
      </div>

      {open ? (
        <div className="rel2__contents">
          {contents.error ? <ErrorNotice error={contents.error} /> : null}
          {contents.isPending ? (
            <SkeletonRows rows={2} height={22} label="Loading what it carried" />
          ) : null}
          <ul>
            {(contents.data?.tasks ?? []).map((task) => (
              <li key={task.task_id}>
                <button type="button" className="rel2__task" onClick={() => openTask(task.task_id)}>
                  <span className="key">{task.key}</span> {task.title}
                </button>
              </li>
            ))}
          </ul>
        </div>
      ) : null}
    </li>
  )
}
