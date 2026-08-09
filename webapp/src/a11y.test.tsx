/**
 * The axe gate (C-019), over rendered output.
 *
 * # Three layers, and what each one actually reaches
 *
 * docs/42 §Accessibility asks for "automated axe checks in CI, plus a manual
 * keyboard-only pass per release" and then says the honest thing: "Automation
 * catches perhaps a third of real issues; the manual pass is where the rest are
 * found." So this file is explicit about its own reach:
 *
 * - `eslint.config.js` reads the **source**. Catches a missing label, a click
 *   handler on a `<div>`, an `aria-*` that does not exist.
 * - This file reads the **rendered DOM**. Catches what only exists after render:
 *   duplicate ids, a `aria-labelledby` pointing at nothing, a landmark
 *   structure that only looks right in JSX, a control whose accessible name
 *   comes out empty.
 * - A human reads the **screen**. Everything else.
 *
 * # What jsdom cannot check, stated rather than implied
 *
 * jsdom has no layout and no rendering, so axe's `color-contrast` rule cannot
 * run here — it needs computed pixels. docs/42 requires 4.5:1 "verified in light
 * and dark from design system tokens", and that verification is **not** in this
 * suite. Nor is focus order, nor scroll containment, nor anything measured.
 * Those need Playwright against a real browser, which is the E2E row docs/15
 * still lists as missing.
 *
 * Saying so here is the point. A suite called "the accessibility gate" that
 * silently skips contrast is worse than no suite, because it retires the
 * question.
 */
import { StrictMode } from 'react'
import { QueryClient, QueryClientProvider } from '@tanstack/react-query'
import { render, cleanup } from '@testing-library/react'
import axe from 'axe-core'
import { afterEach, describe, expect, it } from 'vitest'

import { ErrorNotice, GapNotice } from './shell/notice'
import { Announcer } from './shell/announce'
import { SignIn } from './shell/SignIn'
import { ApiError } from './api/problem'

afterEach(cleanup)

/** A client with retries off: a test must not wait out a backoff schedule. */
function testClient(): QueryClient {
  return new QueryClient({
    defaultOptions: { queries: { retry: false }, mutations: { retry: false } },
  })
}

/**
 * Run axe over a container and return the violations.
 *
 * `color-contrast` is disabled explicitly rather than left to fail silently:
 * axe reports it as "incomplete" under jsdom, which does not fail a test and
 * does not appear in a report — the worst of both. Naming it here means the
 * exclusion is visible in the source of the gate that excludes it.
 */
async function violationsIn(container: Element): Promise<axe.Result[]> {
  const results = await axe.run(container, {
    rules: { 'color-contrast': { enabled: false } },
  })
  return results.violations
}

function describeViolations(violations: readonly axe.Result[]): string {
  return violations
    .map((violation) => `${violation.id}: ${violation.help} (${violation.nodes.length} node(s))`)
    .join('\n')
}

describe('the sign-in screen', () => {
  it('has no axe violations', async () => {
    const { container } = render(
      <StrictMode>
        <QueryClientProvider client={testClient()}>
          <SignIn />
        </QueryClientProvider>
      </StrictMode>,
    )
    const violations = await violationsIn(container)
    expect(describeViolations(violations)).toBe('')
  })

  it('labels both credential fields', async () => {
    // The failure this catches is not hypothetical: a placeholder is not a
    // label, and a form styled with the label visually hidden is one refactor
    // away from having no accessible name at all.
    const { getByLabelText } = render(
      <QueryClientProvider client={testClient()}>
        <SignIn />
      </QueryClientProvider>,
    )
    expect(getByLabelText('Email')).toBeDefined()
    expect(getByLabelText('Password')).toBeDefined()
  })
})

describe('refusals', () => {
  it('render as an alert with no axe violations', async () => {
    const error = new ApiError({
      code: 'TF-CNC-0001',
      status: 409,
      message: 'version conflict',
      requestId: '018f2c00-0000-7000-8000-000000000000',
    })
    const { container, getByRole } = render(<ErrorNotice error={error} />)
    expect(getByRole('alert').textContent).toContain('Someone else changed this first')
    expect(describeViolations(await violationsIn(container))).toBe('')
  })

  it('never show the server’s own message', () => {
    // docs/05 gives every refusal a registry code so the client never has to
    // render a raw body. This is the assertion that keeps that true.
    const error = new ApiError({
      code: 'TF-SYS-0001',
      status: 500,
      message: 'thread panicked at src/tasks/crud.rs:412',
    })
    const { container } = render(<ErrorNotice error={error} />)
    expect(container.textContent).not.toContain('crud.rs')
  })

  it('render a gap as a gap, not as an error', async () => {
    const { container, queryByRole } = render(<GapNotice what="Relations are not readable yet." />)
    // An unbuilt capability is not something that went wrong. Rendering it as an
    // alert teaches users to ignore alert styling.
    expect(queryByRole('alert')).toBeNull()
    // And it says it in the reader's language: no tracker id, no endpoint path.
    expect(container.textContent).toContain('Relations are not readable yet.')
    expect(container.textContent).not.toMatch(/C-0\d\d|\/api\/v1\//)
    expect(describeViolations(await violationsIn(container))).toBe('')
  })
})

describe('the live region', () => {
  it('is a polite, atomic status region that is present before it has anything to say', async () => {
    // Present from mount, deliberately: a region inserted at the moment it gets
    // content is not announced by several screen readers, because they only
    // watch regions that existed when the page settled.
    const { container } = render(
      <Announcer>
        <p>content</p>
      </Announcer>,
    )
    const region = container.querySelector('[role="status"]')
    expect(region).not.toBeNull()
    expect(region?.getAttribute('aria-live')).toBe('polite')
    expect(region?.getAttribute('aria-atomic')).toBe('true')
    expect(describeViolations(await violationsIn(container))).toBe('')
  })
})
