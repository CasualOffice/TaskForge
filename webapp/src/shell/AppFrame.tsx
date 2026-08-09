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

      <header className="shell__top">
        <span className="shell__wordmark">TaskForge</span>
        <WorkspaceSwitcher />
        <span className="shell__spacer" />
        <PaletteHint />
        <ThemeToggle />
        <SignOutButton onSignedOut={forget} />
      </header>

      <ProductRail />


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
 * The product rail.
 *
 * `design/LAYOUT-AND-INTERACTION-GUIDELINES.md` §2 sizes it at 52–60 px and §3
 * keeps it to "a small set of permanent destinations". It replaced a 232 px
 * text sidebar, which spent a fifth of a 1280 px viewport on four words and
 * pushed the board into a narrower column than the content needed — §1.8 now
 * makes that a fault directly: "use the width available".
 *
 * # Why the label is still there
 *
 * An icon-only rail is a memory test. Each destination keeps a visible label
 * under its glyph — the foundation §3 warns against "tiny navigation icons",
 * and §7 requires an accessible name on an icon-only control anyway, so the
 * choice is between a label everyone can read and one only a screen reader
 * gets. 56 px is enough for both at `--tf-meta`.
 *
 * # Search is a command, not a route
 *
 * §3 names Search a permanent destination *and* makes `Cmd/Ctrl+K` "a primary
 * interaction surface for create, jump, assign, transition, search". There is
 * one search in the product and it lives in the palette, so the rail opens the
 * palette rather than routing somewhere that would have to reimplement it. The
 * rail entry exists because the same section warns that "command palette
 * capability must not justify hiding essential discoverable actions".
 */
function ProductRail(): ReactElement {
  return (
    <nav className="rail" aria-label="Primary">
      <Link to="/my-work" className="rail__mark" aria-label="TaskForge — My Work">
        {/* The mark, per VISUAL-IDENTITY §3 ("app rail") and §6 (24–32 px).
            An <img> rather than an inline SVG: it is the same file the favicon
            uses, so the two cannot drift into two different marks. */}
        <img src="/brand/taskforge-mark.svg" alt="" width={26} height={26} />
      </Link>

      <ul className="rail__group">
        <RailLink to="/my-work" label="My Work" icon={<IconMyWork />} />
        <RailLink to="/" label="Tasks" icon={<IconTasks />} exact />
        <RailLink to="/board" label="Board" icon={<IconBoard />} />
        <RailSearch />
        <RailLink to="/reports" label="Reports" icon={<IconReports />} />
      </ul>
    </nav>
  )
}

function RailLink({
  to,
  label,
  icon,
  exact = false,
}: {
  to: string
  label: string
  icon: ReactElement
  exact?: boolean
}): ReactElement {
  return (
    <li>
      {/* `activeProps` sets `aria-current`, not just a class: the styling says
          "this one" to sighted users and the attribute says it to everyone
          else (WCAG 2.2 §2.4.8). */}
      <Link
        to={to}
        className="rail__link"
        activeOptions={{ exact }}
        activeProps={{ 'aria-current': 'page' }}
      >
        <span className="rail__icon" aria-hidden="true">
          {icon}
        </span>
        <span className="rail__label">{label}</span>
      </Link>
    </li>
  )
}

/**
 * Opens the command palette.
 *
 * Dispatches the shortcut rather than lifting the palette's open state: that
 * state belongs to `CommandPalette`, which owns the keyboard contract, and
 * routing it through a shared store would give the same behaviour two owners.
 * The synthetic event goes to the same `window` listener a real keypress does,
 * so there is exactly one code path into the palette.
 */
function RailSearch(): ReactElement {
  return (
    <li>
      <button
        type="button"
        className="rail__link"
        onClick={() =>
          window.dispatchEvent(new KeyboardEvent('keydown', { key: 'k', metaKey: true }))
        }
      >
        <span className="rail__icon" aria-hidden="true">
          <IconSearch />
        </span>
        <span className="rail__label">Search</span>
      </button>
    </li>
  )
}

/* The rail's glyphs.
 *
 * Inline, and drawn here rather than pulled from an icon package: ADR-024
 * budgets the initial shell, and five paths are cheaper than any library's
 * entry point. `stroke="currentColor"` so a destination's colour is decided by
 * the link's state in CSS, not by five copies of a hex value.
 *
 * 20 px is the foundation §3 navigation size; the 32 px target around it comes
 * from `.rail__link`, because §3 also says not to make the glyph the size of
 * its interaction target. */
const GLYPH = {
  width: 20,
  height: 20,
  viewBox: '0 0 20 20',
  fill: 'none',
  stroke: 'currentColor',
  strokeWidth: 1.6,
  strokeLinecap: 'round',
  strokeLinejoin: 'round',
} as const

function IconMyWork(): ReactElement {
  return (
    <svg {...GLYPH}>
      <circle cx="10" cy="6" r="3" />
      <path d="M4 16.5a6 6 0 0 1 12 0" />
    </svg>
  )
}

function IconTasks(): ReactElement {
  return (
    <svg {...GLYPH}>
      <path d="M3 5.5h2l1.5 1.5L9 4.5" />
      <path d="M3 13.5h2L6.5 15 9 12.5" />
      <path d="M11.5 6h5.5M11.5 14h5.5" />
    </svg>
  )
}

function IconBoard(): ReactElement {
  return (
    <svg {...GLYPH}>
      <rect x="3" y="3.5" width="4.5" height="13" rx="1" />
      <rect x="9.5" y="3.5" width="4.5" height="8.5" rx="1" />
      <path d="M16 3.5h1" />
    </svg>
  )
}

function IconSearch(): ReactElement {
  return (
    <svg {...GLYPH}>
      <circle cx="9" cy="9" r="5" />
      <path d="M12.8 12.8 17 17" />
    </svg>
  )
}

function IconReports(): ReactElement {
  return (
    <svg {...GLYPH}>
      <path d="M3 16.5h14" />
      <path d="M6 16.5V10M10 16.5V5M14 16.5v-4" />
    </svg>
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
