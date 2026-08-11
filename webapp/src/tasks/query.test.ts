/**
 * The address bar, both ways.
 *
 * # Why the round trip is the test
 *
 * `filterFromSearch` and `searchFromFilter` are two hand-written translations
 * of the same table, and three of the names differ between the two vocabularies
 * — `due_at`/`due`, `is_blocked`/`blocked`, `created_at`/`created`. A test that
 * checked each direction against its own expectations would pass happily while
 * the pair disagreed, which is the exact failure that matters: a dashboard tile
 * counting overdue work and a list link that drops the date clause, so the
 * number says 3 and the page it opens says 47.
 *
 * So the assertion is that the pair composes to identity. It is what makes a
 * tile's count and the rows behind it the same question.
 */
import { describe, expect, it } from 'vitest'

import { filterFromSearch, searchFromFilter } from './query'
import type { TaskFilter } from '../api/tasks'
import type { AppSearch } from '../router'

/** Every filter a built-in dashboard tile actually sends, and then some. */
const FILTERS: readonly TaskFilter[] = [
  { state: 'BACKLOG,PLANNED,ACTIVE' },
  { state: 'BACKLOG,PLANNED,ACTIVE', due_at: '<@today' },
  { assignee: '@me', state: 'BACKLOG,PLANNED,ACTIVE', due_at: '@today..+7d' },
  { state: 'BACKLOG,PLANNED,ACTIVE', is_blocked: 'true' },
  // The three where present-and-empty is the grammar's `is_empty`, and the
  // single most valuable number on a workload dashboard.
  { state: 'BACKLOG,PLANNED,ACTIVE', assignee: '' },
  { state: 'BACKLOG,PLANNED,ACTIVE', team: '' },
  { type: 'BUG,INCIDENT', priority: 'URGENT', state: 'BACKLOG,PLANNED,ACTIVE' },
  { project: 'p1', created_at: '>-30d', updated_at: '<@today', title: 'drag' },
  { q: 'board', tag: '', parent: '', archived: 'true', reporter: '@me' },
]

describe('a filter and the address that produces it', () => {
  for (const filter of FILTERS) {
    it(`round-trips ${JSON.stringify(filter)}`, () => {
      expect(filterFromSearch(searchFromFilter(filter) as AppSearch)).toEqual(filter)
    })
  }

  it('keeps an empty value, because `field=` means is_empty', () => {
    // Dropping it would turn "unassigned work" into "all work" — a link that
    // silently widens its own filter is worse than a link that fails.
    expect(searchFromFilter({ assignee: '' })).toEqual({ assignee: '' })
    expect(searchFromFilter({ team: '' })).toEqual({ team: '' })
  })

  it('renames the three fields whose two vocabularies differ', () => {
    // Stated explicitly as well as via the round trip: if both directions were
    // wrong in the same way the round trip would still pass.
    expect(searchFromFilter({ due_at: '<@today' })).toEqual({ due: '<@today' })
    expect(searchFromFilter({ is_blocked: 'true' })).toEqual({ blocked: 'true' })
    expect(searchFromFilter({ created_at: '>-7d' })).toEqual({ created: '>-7d' })
  })

  it('drops the fields no address bar can carry', () => {
    // `key`, `milestone` and `environment` have no search parameter. Emitting
    // them would put `?milestone=…` in a URL the router validates away, and a
    // link that loses part of its filter shows the wrong rows under the right
    // heading.
    expect(searchFromFilter({ key: 'WEB-1', milestone: 'm1', environment: 'e1' })).toEqual({})
  })
})
