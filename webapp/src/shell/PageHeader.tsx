/**
 * The band every view starts with: where you are, what this is, and the one
 * action it exists for.
 *
 * # The layout problem this solves
 *
 * Every view began with a row of dropdowns. Nothing said which project you were
 * looking at, nothing said how you got there, and the first thing the eye met
 * was a control rather than a subject — which is why the product read as a
 * tool-bar with a table under it rather than as a page about something.
 *
 * Three lines, in the order a reader needs them:
 *
 * 1. **Breadcrumb** — the workspace, the project when one is scoped, and the
 *    view. It is how you get *out*, which a product with no back button
 *    otherwise leaves to the browser.
 * 2. **Title and count**, with the primary action opposite. One action, on the
 *    right, at the height of the title: the shape every product this one is
 *    measured against uses, because it is where the eye lands after reading the
 *    title.
 * 3. **Tabs**, when a project is scoped — the project's own views.
 *
 * # Why the breadcrumb is not a router thing
 *
 * TanStack can produce a trail from the matched routes, and that trail would be
 * `/ → board`: the route tree is flat because the project lives in the query
 * string, not the path. The crumbs people want are workspace → project → view,
 * which is the *scope* chain and not the route chain. Deriving it from the
 * scope is the honest version.
 */
import type { ReactElement, ReactNode } from 'react'
import { Link } from '@tanstack/react-router'
import { useQuery } from '@tanstack/react-query'

import { keys } from '../api/keys'
import { listProjects } from '../api/projects'
import { useAppSearch } from './navigation'
import { useSession, useWorkspaceId } from './session'

/** The project's views. Kept here so every view agrees on the set and order. */
const TABS: ReadonlyArray<{ to: string; label: string; exact?: boolean }> = [
  { to: '/board', label: 'Board' },
  { to: '/', label: 'List', exact: true },
  { to: '/reports', label: 'Reports' },
]

export function PageHeader({
  title,
  count,
  actions,
  children,
}: {
  title: string
  /** Rendered beside the title — "12 shown", "3 overdue". */
  count?: ReactNode
  /** The view's primary action. One, or none. */
  actions?: ReactNode
  /** The heading's id, so the view's `aria-labelledby` still resolves. */
  children?: ReactNode
}): ReactElement {
  const { workspace } = useSession()
  const workspaceId = useWorkspaceId()
  const search = useAppSearch()

  const projects = useQuery({
    queryKey: keys.projects(workspaceId),
    queryFn: ({ signal }) => listProjects(workspaceId, signal),
    enabled: workspaceId !== '',
    staleTime: 60_000,
  })
  const project = (projects.data?.data ?? []).find((entry) => entry.id === search.project)

  return (
    <header className="page">
      <nav className="page__crumbs" aria-label="Breadcrumb">
        <ol>
          <li>{workspace?.name ?? 'Workspace'}</li>
          {project === undefined ? null : (
            <li>
              <Link to="/board" search={{ project: project.id }}>
                {project.name}
              </Link>
            </li>
          )}
          <li aria-current="page">{title}</li>
        </ol>
      </nav>

      <div className="page__head">
        {/* The page's only `h1`. Views point their `aria-labelledby` at this id
            rather than carrying a second, visually-hidden heading of their own —
            two `h1`s describing one page is a heading structure that reads as
            two pages to anyone navigating by headings. */}
        <h1 className="page__title" id="page-title">
          {title}
        </h1>
        {count === undefined ? null : <span className="page__count">{count}</span>}
        <span className="page__spacer" />
        {actions}
      </div>

      {/* The project's own views. Absent when nothing is scoped, because
          "Board" and "List" across every project at once are the workspace's
          views, not a project's, and tabs would imply otherwise. */}
      {project === undefined ? null : (
        <nav className="page__tabs" aria-label={`${project.name} views`}>
          {TABS.map((tab) => (
            <Link
              key={tab.to}
              to={tab.to}
              search={{ project: project.id }}
              className="page__tab"
              activeOptions={{ exact: tab.exact ?? false }}
              activeProps={{ 'aria-current': 'page' }}
            >
              {tab.label}
            </Link>
          ))}
        </nav>
      )}

      {children}
    </header>
  )
}
