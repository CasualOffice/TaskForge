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

/**
 * Two groups, because there are exactly two scopes (`docs/49` §3).
 *
 * A person needs to know whether they are about to change something for
 * themselves or for everybody, and one flat list could not say it. The headings
 * are the whole mechanism: *Account* affects only you, *Workspace* affects
 * everyone and mostly needs a permission.
 *
 * The descriptions that used to sit under every entry are gone from here. They
 * belong on the page they describe, not in the list you scan to find it — seven
 * entries of two lines each is a wall of text where a menu should be.
 */
const GROUPS: ReadonlyArray<{
  heading: string
  sections: ReadonlyArray<{ to: string; label: string }>
}> = [
  {
    heading: 'Account',
    sections: [{ to: '/settings/profile', label: 'Your profile' }],
  },
  {
    heading: 'Workspace',
    sections: [
      { to: '/settings/workspace', label: 'General' },
      { to: '/settings/members', label: 'Members' },
      { to: '/settings/teams', label: 'Teams' },
      { to: '/settings/roles', label: 'Roles' },
      { to: '/settings/workflow', label: 'Workflow' },
      { to: '/settings/environments', label: 'Environments' },
      { to: '/settings/tags', label: 'Tags' },
    ],
  },
]

export function SettingsLayout(): ReactElement {
  return (
    <div className="settings">
      <nav className="settings__nav" aria-label="Settings">
        {/* A label, not an `<h1>`. The heading belongs to the page, which is
            what names the section a reader is actually on — every settings
            route used to have "Settings" as its only heading, so no page ever
            said what it was (`docs/49` §1). */}
        <p className="settings__title">Settings</p>
        {GROUPS.map((group) => (
          <div className="settings__group" key={group.heading}>
            <h2 className="settings__grouphead" id={`settings-${group.heading}`}>
              {group.heading}
            </h2>
            <ul className="settings__list" aria-labelledby={`settings-${group.heading}`}>
              {group.sections.map((section) => (
                <li key={section.to}>
                  <Link
                    to={section.to}
                    className="settings__link"
                    // Exactly, so one entry is current and never two.
                    activeOptions={{ exact: true }}
                    activeProps={{ 'aria-current': 'page' }}
                  >
                    {section.label}
                  </Link>
                </li>
              ))}
            </ul>
          </div>
        ))}
      </nav>
      <div className="settings__panel">
        <Outlet />
      </div>
    </div>
  )
}
