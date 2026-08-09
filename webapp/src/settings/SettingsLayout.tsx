/**
 * The settings shell: one sub-navigation, one panel.
 *
 * # Why settings is a route tree and not a modal
 *
 * Every section here is a place a person is sent to — "your admin can change
 * that under Roles" is only useful if Roles has an address. A modal cannot be
 * linked, cannot be opened in a second tab beside the thing being configured,
 * and loses its position on reload.
 *
 * # Why the entries are not filtered by permission
 *
 * A section the caller cannot administer renders the sentence that says so,
 * naming the permission. Hiding the entry instead would leave someone who was
 * told "change it under Roles" staring at a navigation without a Roles in it,
 * with nothing to search for and no way to know whether the feature exists or
 * they simply cannot see it. Presence is discoverability; the refusal inside is
 * the honest part.
 */
import type { ReactElement } from 'react'
import { Link, Outlet } from '@tanstack/react-router'

const SECTIONS: ReadonlyArray<{ to: string; label: string; detail: string }> = [
  { to: '/settings/profile', label: 'Your profile', detail: 'Name, time zone, password, sessions' },
  { to: '/settings/workspace', label: 'Workspace', detail: 'Name and identity' },
  { to: '/settings/members', label: 'Members', detail: 'People and invitations' },
  { to: '/settings/teams', label: 'Teams', detail: 'Groups a grant can name' },
  { to: '/settings/roles', label: 'Roles', detail: 'What each role carries, and who holds it' },
  { to: '/settings/workflow', label: 'Workflow', detail: 'Statuses and the moves between them' },
  { to: '/settings/tags', label: 'Tags', detail: 'The shared vocabulary' },
]

export function SettingsLayout(): ReactElement {
  return (
    <div className="settings">
      <nav className="settings__nav" aria-label="Settings">
        <h1 className="settings__title">Settings</h1>
        <ul className="settings__list">
          {SECTIONS.map((section) => (
            <li key={section.to}>
              <Link
                to={section.to}
                className="settings__link"
                activeProps={{ 'aria-current': 'page' }}
              >
                <span className="settings__link-label">{section.label}</span>
                <span className="settings__link-detail">{section.detail}</span>
              </Link>
            </li>
          ))}
        </ul>
      </nav>
      <div className="settings__panel">
        <Outlet />
      </div>
    </div>
  )
}
