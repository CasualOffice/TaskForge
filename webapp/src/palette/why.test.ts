import { describe, expect, it } from 'vitest'

import { whyMatched, type People } from './why'

const PEOPLE: People = new Map([
  ['u-1', 'Ash Bekele'],
  ['u-2', 'Rin Okafor'],
])

function task(overrides: Partial<Parameters<typeof whyMatched>[0]> = {}) {
  return {
    title: 'Backup restore drill',
    description: null,
    reporter_id: 'u-1',
    assignees: [],
    ...overrides,
  }
}

describe('why a task matched', () => {
  it('says nothing when the title already shows it', () => {
    // The failure this forbids: a note on every row. A subtitle that is always
    // there is a subtitle nobody reads, which defeats the one case it exists for.
    expect(whyMatched(task(), 'backup', PEOPLE)).toBeUndefined()
  })

  it('names the reporter, which is the case that looked broken', () => {
    // Weight B indexes the reporter's display name on every task they raise, so
    // searching a colleague returns all of their work with no visible reason.
    expect(whyMatched(task(), 'Ash', PEOPLE)).toBe('reported by Ash Bekele')
  })

  it('prefers the assignee over the reporter', () => {
    // Both can match at once. "Assigned to" is the more useful of the two —
    // whose work it is now beats who raised it a year ago.
    const both = task({ reporter_id: 'u-1', assignees: ['u-1'] })
    expect(whyMatched(both, 'Ash', PEOPLE)).toBe('assigned to Ash Bekele')
  })

  it('names the description', () => {
    const documented = task({ description: 'A backup nobody has restored is a hope, not a backup.' })
    expect(whyMatched(documented, 'hope', PEOPLE)).toBe('matches the description')
  })

  it('does not claim a place it cannot see', () => {
    // A comment body, a tag, or a stemmed form: all indexed, none on the row.
    // Guessing "in a comment" would be a confident wrong answer.
    expect(whyMatched(task(), 'restoring', PEOPLE)).toBe('matches elsewhere in the task')
  })

  it('is silent for an empty query', () => {
    expect(whyMatched(task(), '   ', PEOPLE)).toBeUndefined()
  })

  it('does not match a person who is not on the task', () => {
    // The failure this forbids: reading the whole member list rather than the
    // ids on this row, and telling somebody a task is Rin's because Rin exists.
    expect(whyMatched(task(), 'Rin', PEOPLE)).toBe('matches elsewhere in the task')
  })
})
