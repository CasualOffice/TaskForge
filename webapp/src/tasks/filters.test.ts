/**
 * The filter layer, guarded.
 *
 * # What is worth testing here and what is not
 *
 * None of this validates a filter — the server does that, and a client-side
 * validator would be the second grammar `docs/27` §Compilation exists to
 * prevent. What these tests guard is the *translation*: the address bar to a
 * query, and back to a sentence. Both are pure, both are subtle, and both fail
 * silently — a dropped clause returns more rows than the user asked for, which
 * is the dangerous direction, and a mis-read chip tells them a filter they do
 * not have.
 *
 * `docs/27` §Acceptance gates asks for a round-trip property: "URL → AST → URL
 * is stable". The client half of that is view → search → view, and it is the
 * last test in this file.
 */
import { describe, expect, it } from 'vitest'

import type { AppSearch } from '../router'
import { BUILTIN_VIEWS, activeView } from './builtinViews'
import { filterFromSearch, hasFilters } from './query'
import { describeFilters } from '../views/filters/describe'

const labels = {
  status: (id: string) => (id === 'st-1' ? 'In Progress' : id),
  person: (id: string) => (id === 'u-1' ? 'Ada' : id),
}

describe('filterFromSearch', () => {
  it('passes the grammar through untranslated', () => {
    // The server's spelling is the client's spelling. A dialect here would be
    // the second grammar docs/27 exists to prevent.
    const filter = filterFromSearch({
      priority: 'HIGH,URGENT',
      state: '!COMPLETED,CANCELED',
      due: '<@today',
      q: 'retry',
    })
    expect(filter).toEqual({
      priority: 'HIGH,URGENT',
      state: '!COMPLETED,CANCELED',
      due_at: '<@today',
      q: 'retry',
    })
  })

  it('maps the URL parameter names onto the grammar field names', () => {
    // `due` / `created` / `updated` are the URL's short names; the grammar's
    // fields are `due_at` / `created_at` / `updated_at`. Getting this wrong is a
    // 400 naming a field the user never typed.
    const filter = filterFromSearch({ due: '<@today', created: '>-7d', updated: '>-1d' })
    expect(Object.keys(filter).sort()).toEqual(['created_at', 'due_at', 'updated_at'])
  })

  it('keeps a present-and-empty assignee, tag and parent', () => {
    // `field=` is the grammar's `is_empty` (docs/27 §URL form). Dropping it the
    // way every other empty value is dropped turns "show me unassigned work"
    // into "show me everything" — a filter that silently widens.
    expect(filterFromSearch({ assignee: '' })).toEqual({ assignee: '' })
    expect(filterFromSearch({ tag: '' })).toEqual({ tag: '' })
    expect(filterFromSearch({ parent: '' })).toEqual({ parent: '' })
  })

  it('drops every other empty value', () => {
    // `priority=` would be `is_empty` on a field whose operators do not include
    // it — a 400 the user could not have caused deliberately.
    expect(filterFromSearch({ priority: '', q: '', state: '', title: '' })).toEqual({})
  })

  it('never emits a key whose value is undefined', () => {
    // `{ priority: undefined }` reaches the query string as
    // `priority=undefined`, which is a literal the enum does not have.
    const filter = filterFromSearch({ project: 'p-1' })
    expect(Object.values(filter).every((value) => value !== undefined)).toBe(true)
  })

  it('does not treat the open drawer as a filter', () => {
    // `task` says which drawer is open, not which rows to fetch. Sending it
    // would be an unknown field and a 400 on every drawer open.
    expect(filterFromSearch({ task: 't-1', project: 'p-1' })).toEqual({ project: 'p-1' })
  })
})

describe('hasFilters', () => {
  it('is false for a bare project scope', () => {
    // A project is scope, not a filter — "Clear" preserves it deliberately.
    expect(hasFilters({ project: 'p-1' })).toBe(false)
  })

  it('is true for unassigned, which is an empty string', () => {
    expect(hasFilters({ assignee: '' })).toBe(true)
  })
})

describe('describeFilters', () => {
  it('reads the grammar back as English rather than as its spelling', () => {
    const [chip] = describeFilters({ state: '!COMPLETED,CANCELED' }, labels)
    expect(chip?.field).toBe('State')
    expect(chip?.value).toBe('not Completed or Canceled')
  })

  it('names the ordered comparison on priority', () => {
    // `priority` is the one ordered enum, so `>=` is a range and not a set.
    const [chip] = describeFilters({ priority: '>=HIGH' }, labels)
    expect(chip?.value).toBe('High or above')
  })

  it('distinguishes unassigned from a named person', () => {
    expect(describeFilters({ assignee: '' }, labels)[0]?.value).toBe('Unassigned')
    expect(describeFilters({ assignee: '@me' }, labels)[0]?.value).toBe('Me')
    expect(describeFilters({ assignee: 'u-1' }, labels)[0]?.value).toBe('Ada')
  })

  it('resolves a status id to the workflow’s own name', () => {
    expect(describeFilters({ status: 'st-1' }, labels)[0]?.value).toBe('In Progress')
  })

  it('puts relative dates into words', () => {
    expect(describeFilters({ updated: '>-7d' }, labels)[0]?.value).toBe('after 7 days ago')
    expect(describeFilters({ due: '@tomorrow..+14d' }, labels)[0]?.value).toBe(
      'tomorrow to in 14 days',
    )
  })

  it('produces one chip per clause and no more', () => {
    // A chip removes exactly one search parameter, so a chip that described two
    // clauses would remove a constraint the user did not point at.
    const chips = describeFilters(
      { q: 'a', priority: 'HIGH', type: 'BUG', assignee: '@me' },
      labels,
    )
    expect(chips).toHaveLength(4)
    expect(new Set(chips.map((chip) => chip.key)).size).toBe(4)
  })
})

describe('the built-in views', () => {
  it('are the seven docs/27 §Built-in views ships', () => {
    expect(BUILTIN_VIEWS.map((view) => view.id)).toEqual([
      'my-today',
      'my-overdue',
      'my-upcoming',
      'my-blocked',
      'my-recently-completed',
      'reported-by-me',
      'unassigned',
    ])
  })

  it('spell "due on or before today" as `<@tomorrow`', () => {
    // `due_at` permits `before after between is_empty` and NOT `lte`, so `<=`
    // has no spelling in the URL form. "Before tomorrow" is the same set of
    // instants and is the only expressible one.
    const today = BUILTIN_VIEWS.find((view) => view.id === 'my-today')
    expect(today?.search.due).toBe('<@tomorrow')
  })

  it('keep symbols symbolic', () => {
    // docs/27: "@me is what makes one saved view correct for every user who
    // opens it. A view that hardcoded a user id would be shareable but wrong."
    // A resolved uuid appearing here would be that bug.
    for (const view of BUILTIN_VIEWS) {
      const values = Object.values(view.search).filter((value) => typeof value === 'string')
      expect(values.some((value) => /^[0-9a-f]{8}-[0-9a-f]{4}/.test(value))).toBe(false)
    }
  })

  it('state only what they constrain', () => {
    // Every key a view sets must be a filter parameter — a view that set
    // `project` would silently move the user to another project.
    for (const view of BUILTIN_VIEWS) {
      expect(Object.keys(view.search)).not.toContain('project')
      expect(Object.keys(view.search)).not.toContain('task')
    }
  })

  it('round-trip: applying a view is recognised as that view', () => {
    // The client half of docs/27's "URL → AST → URL is stable". If this fails,
    // the menu applies a filter and then cannot tell you which view you are on.
    for (const view of BUILTIN_VIEWS) {
      const applied = view.search as AppSearch
      expect(activeView(applied)?.id).toBe(view.id)
    }
  })

  it('round-trip survives an unrelated project scope and open drawer', () => {
    // A built-in view scoped to a project is still that view, and an open
    // drawer is not part of the filter.
    const overdue = BUILTIN_VIEWS.find((view) => view.id === 'my-overdue')
    const applied = { ...overdue?.search, project: 'p-1', task: 't-1' } as AppSearch
    expect(activeView(applied)?.id).toBe('my-overdue')
  })

  it('does not claim a view for a filter that merely overlaps one', () => {
    // "assignee=@me" alone is not "Overdue". Matching loosely would label a
    // user's own ad-hoc filter as a named view and then change it under them.
    expect(activeView({ assignee: '@me' })).toBeUndefined()
  })

  it('only Blocked is still withheld', () => {
    const withheld = BUILTIN_VIEWS.filter((view) => view.unavailable !== undefined)
    expect(withheld.map((view) => view.id)).toEqual(['my-blocked'])
  })
})
