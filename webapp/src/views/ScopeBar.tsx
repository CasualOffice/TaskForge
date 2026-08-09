/**
 * What a view is looking at: a project, and a search term.
 *
 * # The failure this module prevents
 *
 * Scope that lives in component state. A project chosen in a `useState` is lost
 * on reload, cannot be linked to, and — worst — disagrees with the board when
 * the user switches views. Both controls here read and write the URL, so the
 * scope *is* the address: the list, the board, and a link pasted into a chat all
 * mean the same thing.
 *
 * # Why the search box is debounced and the project picker is not
 *
 * Typing produces a request per keystroke; choosing a project produces one per
 * decision. Debouncing the picker would add latency to an action that has none
 * to hide.
 */
import { useEffect, useState, type ReactElement } from 'react'
import { useQuery } from '@tanstack/react-query'

import { keys } from '../api/keys'
import { listProjects } from '../api/projects'
import { useAppSearch, useUpdateSearch } from '../shell/navigation'
import { useWorkspaceId } from '../shell/session'

/** Long enough that a typist does not fire a request per letter, short enough to feel live. */
const DEBOUNCE_MS = 250

export function ScopeBar({ children }: { children?: ReactElement | null }): ReactElement {
  const workspaceId = useWorkspaceId()
  const search = useAppSearch()
  const update = useUpdateSearch()

  const projects = useQuery({
    queryKey: keys.projects(workspaceId),
    queryFn: ({ signal }) => listProjects(workspaceId, signal),
    enabled: workspaceId !== '',
    staleTime: 60_000,
  })

  const [term, setTerm] = useState(search.q ?? '')

  // The URL is the authority: a back button or a pasted link changes `search.q`
  // without going through this input, and a box that ignored that would show one
  // term while the list showed another.
  useEffect(() => setTerm(search.q ?? ''), [search.q])

  useEffect(() => {
    const current = search.q ?? ''
    if (term === current) return
    const timer = setTimeout(() => update({ q: term === '' ? undefined : term }), DEBOUNCE_MS)
    return () => clearTimeout(timer)
  }, [term, search.q, update])

  return (
    <div className="view__bar">
      <label className="visually-hidden" htmlFor="scope-project">
        Project
      </label>
      <select
        id="scope-project"
        className="select scope__select"
        value={search.project ?? ''}
        onChange={(event) =>
          update({ project: event.target.value === '' ? undefined : event.target.value })
        }
      >
        <option value="">All projects</option>
        {(projects.data?.data ?? []).map((project) => (
          <option key={project.id} value={project.id}>
            {project.key} — {project.name}
          </option>
        ))}
      </select>

      <label className="visually-hidden" htmlFor="scope-search">
        Search tasks
      </label>
      <input
        id="scope-search"
        className="input scope__search"
        type="search"
        placeholder="Search…"
        value={term}
        onChange={(event) => setTerm(event.target.value)}
      />

      {children}
    </div>
  )
}
