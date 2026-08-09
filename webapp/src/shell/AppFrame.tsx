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
import { useMutation, useQueryClient } from '@tanstack/react-query'

import { logout } from '../api/session'
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

      <div className="shell__brand">TaskForge</div>

      <header className="shell__top">
        <WorkspaceSwitcher />
        <span className="shell__spacer" />
        <PaletteHint />
        <ThemeToggle />
        <SignOutButton onSignedOut={forget} />
      </header>

      <nav className="shell__nav" aria-label="Primary">
        <ul className="nav__group">
          <NavLink to="/my-work" label="My Work" />
          <NavLink to="/" label="Tasks" />
          <NavLink to="/board" label="Board" />
          <NavLink to="/reports" label="Reports" />
        </ul>
      </nav>

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

function NavLink({ to, label }: { to: string; label: string }): ReactElement {
  return (
    <li>
      {/* `activeProps` sets `aria-current`, not just a class: the styling says
          "this one" to sighted users and the attribute says it to everyone
          else (WCAG 2.2 §2.4.8). */}
      <Link
        to={to}
        className="nav__link"
        activeOptions={{ exact: to === '/' }}
        activeProps={{ 'aria-current': 'page' }}
      >
        {label}
      </Link>
    </li>
  )
}

function PaletteHint(): ReactElement {
  return (
    <span className="palette-hint" aria-hidden="true">
      <kbd>⌘</kbd>
      <kbd>K</kbd>
    </span>
  )
}

function ThemeToggle(): ReactElement {
  const [choice, setChoice] = useState<ThemeChoice>(() => storedChoice())

  useEffect(() => {
    apply(choice)
  }, [choice])

  const label = choice === 'system' ? 'System theme' : choice === 'light' ? 'Light theme' : 'Dark theme'

  return (
    <button
      type="button"
      className="button button--quiet"
      onClick={() => setChoice(nextChoice(choice))}
      // The button's own text is an icon-free word, but the *state* is what a
      // screen reader needs and "Theme" alone would not carry it.
      aria-label={`${label}. Activate to change.`}
    >
      {label}
    </button>
  )
}

function SignOutButton({ onSignedOut }: { onSignedOut: () => void }): ReactElement {
  const client = useQueryClient()
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

  return (
    <button
      type="button"
      className="button button--quiet"
      onClick={() => signOut.mutate()}
      disabled={signOut.isPending}
    >
      Sign out
    </button>
  )
}
