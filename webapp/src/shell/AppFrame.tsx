/**
 * The authenticated frame: brand, navigation, workspace, and the palette key.
 *
 * # The failure this module prevents
 *
 * Navigation that grows one entry per feature. docs/42 §Command palette: "it is
 * why permanent navigation can stay at seven items — new capability adds a
 * command, not a nav entry." So this file has a fixed, short list, and anything
 * that wants to be reachable registers a command instead. The rule only holds if
 * the nav is somewhere a contributor has to argue to change; a nav assembled
 * from a registry every view could append to would not be.
 *
 * # Why it renders three different things
 *
 * Loading, signed-out, and signed-in are genuinely different screens, not one
 * screen with fields missing. Rendering the frame while the session is unknown
 * produces a flash of an app the visitor may not be allowed into, and rendering
 * it with no workspace produces a nav whose every link 404s.
 */
import { useEffect, useState, type ReactElement } from 'react'
import { Link, Outlet } from '@tanstack/react-router'
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query'

import { Avatar, Button, Icon, Kbd } from '@schnsrw/design-system'

import { keys } from '../api/keys'
import { readMe } from '../api/me'
import { listProjects } from '../api/projects'
import { logout } from '../api/session'
import { Popover } from './Popover'
import { useAppSearch, useUpdateSearch } from './navigation'
import { useWorkspaceId } from './session'
import { CommandPalette } from '../palette/CommandPalette'
import { SignIn } from './SignIn'
import { WorkspaceSwitcher } from './WorkspaceSwitcher'
import { useSession } from './session'
import { apply, nextChoice, storedChoice, type ThemeChoice } from './theme'

export function AppFrame(): ReactElement {
  const { actor, loading, workspace, forget } = useSession()

  if (loading) {
    // A `role="status"` rather than a spinner: the first thing a screen-reader
    // user should hear is that something is happening, not silence.
    return (
      <main className="empty" role="status">
        Loading TaskForge…
      </main>
    )
  }

  if (actor === null) return <SignIn />

  return (
    <div className="shell">
      <a className="skip-link" href="#main">
        Skip to content
      </a>

      <header className="shell__top">
        {/* The mark lives here, with the wordmark: a product's identity belongs
            on the bar that spans the window. It is the same file the favicon
            uses, so the two cannot drift into two different marks. */}
        <Link to="/home" className="shell__brand" aria-label="TaskForge — Home">
          <img src="/brand/taskforge-mark.svg" alt="" width={22} height={22} />
          <span className="shell__wordmark">TaskForge</span>
        </Link>
        <SearchButton />
        <span className="shell__spacer" />
        <ThemeToggle />
        <AccountMenu onSignedOut={forget} />
      </header>

      <Sidebar />

      <main className="shell__main" id="main">
        {workspace === undefined ? (
          <p className="empty">
            You are signed in but belong to no workspace yet. Ask an owner for an invitation.
          </p>
        ) : (
          <Outlet />
        )}
      </main>

      <CommandPalette />
    </div>
  )
}

/**
 * The sidebar: where you are, and where else you can go.
 *
 * # Why this replaced a 60 px icon rail
 *
 * The rail fitted a glyph with a two-word label stacked under it, and the
 * result read as a strip of tiny icons — the pattern every product this one is
 * measured against replaced years ago. At 240 px a destination is a row: icon
 * beside label, sections with headings, and room for the *project* navigation
 * that an icon rail had nowhere to put.
 *
 * # Projects are navigation, not a filter
 *
 * A project used to be reachable only through a dropdown on the toolbar, so
 * "open the Platform board" meant landing on a board and then narrowing it.
 * Here a project is a place: choosing one sets the scope every view already
 * reads from the URL, and the views under it are that project's views. The
 * scope still lives in `?project=`, so a link is still a link and nothing about
 * the query grammar changed — what changed is that there is now a door.
 *
 * # Why the workspace switcher sits at the top
 *
 * It is the outermost scope, and everything below it is inside that scope.
 * Putting it in the header, beside the product mark, said the opposite: that
 * the workspace is a property of the application rather than the container for
 * the navigation under it.
 */
function Sidebar(): ReactElement {
  const search = useAppSearch()
  const update = useUpdateSearch()
  const workspaceId = useWorkspaceId()

  const projects = useQuery({
    queryKey: keys.projects(workspaceId),
    queryFn: ({ signal }) => listProjects(workspaceId, signal),
    enabled: workspaceId !== '',
    staleTime: 60_000,
  })

  const all = projects.data?.data ?? []
  const current = all.find((project) => project.id === search.project)

  return (
    <nav className="side" aria-label="Primary">
      <div className="side__scope">
        <WorkspaceSwitcher />
      </div>

      <ul className="side__group">
        <SideLink to="/home" label="Home" icon="home" />
        <SideLink to="/my-work" label="My work" icon="person" />
        <SideLink to="/" label="All tasks" icon="checklist" exact />
        <SideLink to="/board" label="Board" icon="view_kanban" />
        <SideLink to="/environments" label="Environments" icon="lan" />
        <SideLink to="/reports" label="Reports" icon="monitoring" />
      </ul>

      {all.length === 0 ? null : (
        <>
          <h2 className="side__heading" id="side-projects">
            Projects
          </h2>
          <ul className="side__group" aria-labelledby="side-projects">
            {all.slice(0, PROJECTS_SHOWN).map((project) => (
              <li key={project.id}>
                <button
                  type="button"
                  className="side__link"
                  // `aria-current` and not a class alone: the styling says
                  // "this one" to sighted users and the attribute says it to
                  // everyone else (WCAG 2.2 §2.4.8).
                  aria-current={project.id === search.project ? 'true' : undefined}
                  onClick={() => update({ project: project.id })}
                >
                  <span className="side__key" aria-hidden="true">
                    {project.key}
                  </span>
                  <span className="side__label">{project.name}</span>
                </button>
              </li>
            ))}
            {current === undefined ? null : (
              <li>
                <button
                  type="button"
                  className="side__link side__link--quiet"
                  onClick={() => update({ project: undefined })}
                >
                  <span className="side__label">Clear project scope</span>
                </button>
              </li>
            )}
          </ul>
          {all.length > PROJECTS_SHOWN ? (
            <p className="side__more">
              {all.length - PROJECTS_SHOWN} more — use search to reach them
            </p>
          ) : null}
        </>
      )}

      <ul className="side__group side__group--foot">
        <SideLink to="/settings" label="Settings" icon="settings" />
      </ul>
    </nav>
  )
}

/** Enough to recognise the workspace, not so many that the sidebar scrolls. */
const PROJECTS_SHOWN = 8

function SideLink({
  to,
  label,
  icon,
  exact = false,
}: {
  to: string
  label: string
  /** A Material Symbols ligature name — the suite's icon convention. */
  icon: string
  exact?: boolean
}): ReactElement {
  return (
    <li>
      <Link
        to={to}
        className="side__link"
        activeOptions={{ exact }}
        activeProps={{ 'aria-current': 'page' }}
      >
        <span className="side__icon" aria-hidden="true">
          <Icon name={icon} size="md" />
        </span>
        <span className="side__label">{label}</span>
      </Link>
    </li>
  )
}

function SearchButton(): ReactElement {
  return (
    <button
      type="button"
      className="shell__search"
      onClick={() =>
        window.dispatchEvent(new KeyboardEvent('keydown', { key: 'k', metaKey: true }))
      }
    >
      {/* Material Symbols, from the design system, rather than a hand-drawn
          path: the suite has one icon convention and a product that draws its
          own magnifier has a magnifier that does not match anyone else's. */}
      <Icon name="search" size="sm" />
      {/* Shaped like the field it opens. A button that looks like a button says
          "something will happen"; one shaped like a search box says what. */}
      <span className="shell__search-text">Search tasks, projects and people</span>
      <Kbd keys="⌘K" size="sm" />
    </button>
  )
}

function ThemeToggle(): ReactElement {
  const [choice, setChoice] = useState<ThemeChoice>(() => storedChoice())

  useEffect(() => {
    apply(choice)
  }, [choice])

  const label =
    choice === 'system' ? 'System theme' : choice === 'light' ? 'Light theme' : 'Dark theme'

  return (
    <Button
      variant="subtle"
      size="sm"
      icon={choice === 'dark' ? 'dark_mode' : choice === 'light' ? 'light_mode' : 'contrast'}
      onClick={() => setChoice(nextChoice(choice))}
      // The label carries the *state*, which is what a screen reader needs;
      // "Theme" alone would not.
      aria-label={`${label}. Activate to change.`}
    >
      {label}
    </Button>
  )
}

/**
 * The account control.
 *
 * An avatar with the person's initials rather than the word "Sign out" sitting
 * permanently in the bar: signing out is the least frequent thing anyone does
 * here, and giving it the most prominent position was a tell that nothing else
 * had claimed one. The initials also answer "who am I signed in as", which the
 * header could not previously say at all.
 */
function AccountMenu({ onSignedOut }: { onSignedOut: () => void }): ReactElement {
  const client = useQueryClient()
  const me = useQuery({ queryKey: keys.me(), queryFn: ({ signal }) => readMe(signal) })
  const signOut = useMutation({
    mutationFn: logout,
    // `onSettled`, not `onSuccess`: if the request failed the credential may
    // still be gone, and leaving a cache full of tenant data behind a possibly
    // dead session is the wrong way to be wrong.
    onSettled: () => {
      onSignedOut()
      client.clear()
    },
  })

  const name = me.data?.display_name ?? ''

  return (
    <Popover
      label={<Avatar name={name} />}
      ariaLabel={name === '' ? 'Your account' : `Your account: ${name}`}
      align="end"
      triggerClass="shell__account"
    >
      {(close) => (
        <div>
          <p className="pop__section">
            <strong>{name === '' ? 'Signed in' : name}</strong>
            {me.data?.email === null || me.data?.email === undefined ? null : (
              <>
                <br />
                {me.data.email}
              </>
            )}
          </p>
          <ul className="pop__list">
            <li>
              <Link to="/settings/profile" className="pop__item" onClick={close}>
                Your profile
              </Link>
            </li>
            <li>
              <button
                type="button"
                className="pop__item"
                disabled={signOut.isPending}
                onClick={() => signOut.mutate()}
              >
                Sign out
              </button>
            </li>
          </ul>
        </div>
      )}
    </Popover>
  )
}
