/**
 * The slug a workspace name suggests.
 *
 * Worth testing because it is the one place the client makes up a value the
 * server will validate, and a suggestion that fails validation is worse than no
 * suggestion: it puts a rejection in front of someone who typed nothing wrong.
 * The server's rule is 1–64 characters of `a-z`, `0-9` and `-`, starting with a
 * letter or digit, so every case below asserts the output would be *accepted*.
 */
import { describe, expect, it } from 'vitest'

import { slugFrom } from './workspaces'

/** `workspaces::support::valid_slug`, transcribed. */
const SERVER_RULE = /^[a-z0-9][a-z0-9-]{0,63}$/

describe('the slug a name suggests', () => {
  const cases: readonly [string, string][] = [
    ['Acme', 'acme'],
    ['Acme, Inc.', 'acme-inc'],
    ['  Spaced  Out  ', 'spaced-out'],
    ['Platform & Infrastructure', 'platform-infrastructure'],
    ['2026 Planning', '2026-planning'],
    ['Ünïcödé Wörks', 'n-c-d-w-rks'],
    ['---leading and trailing---', 'leading-and-trailing'],
  ]

  for (const [name, expected] of cases) {
    it(`turns ${JSON.stringify(name)} into ${JSON.stringify(expected)}`, () => {
      expect(slugFrom(name)).toBe(expected)
    })
  }

  it('never suggests something the server would refuse', () => {
    // The property that matters, over the shapes a name actually takes. A
    // suggestion the server rejects is a rejection someone did not earn.
    for (const [name] of cases) {
      expect(SERVER_RULE.test(slugFrom(name)), `${name} → ${slugFrom(name)}`).toBe(true)
    }
  })

  it('never begins or ends with a separator', () => {
    // The specific way this goes wrong: punctuation at either end becomes a
    // hyphen, and a leading hyphen is the one thing the server names explicitly.
    for (const name of ['.hidden', '!bang', 'trailing!', '  ', '???']) {
      const slug = slugFrom(name)
      expect(slug.startsWith('-'), name).toBe(false)
      expect(slug.endsWith('-'), name).toBe(false)
    }
  })

  it('gives back nothing when a name has nothing to work with', () => {
    // Empty is not a valid slug, and that is correct: the field stays empty,
    // the form stays disabled, and the person types one. Inventing "workspace"
    // would put a name nobody chose into a URL that cannot be changed here.
    expect(slugFrom('???')).toBe('')
    expect(slugFrom('')).toBe('')
  })

  it('stays within the 64 the server accepts', () => {
    expect(slugFrom('a'.repeat(200)).length).toBe(64)
  })
})
