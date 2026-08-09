/**
 * The URL writer, guarded.
 *
 * # The bug this pins
 *
 * `docs/27` §URL form: "`field=` — the empty value is how a URL says 'unset'".
 * Two places have to agree about which parameters mean that — the router's
 * validator, which decides what survives being *read* out of the address bar,
 * and `prune`, which decides what survives being *written* back into it.
 *
 * They disagreed. The validator kept `assignee`, `tag` and `parent`; the writer
 * kept only `assignee`. So `?tag=` (untagged) and `?parent=` (top level) worked
 * when pasted and vanished the moment any other control moved — a filter that
 * holds until you touch the page, which is the worst kind, because the rows it
 * adds back look like data rather than like a bug.
 *
 * Both now read one list. This test fails if a third copy appears.
 */
import { describe, expect, it } from 'vitest'

import { EMPTY_IS_MEANINGFUL } from '../router'
import { prune } from './navigation'

describe('writing the search parameters back to the URL', () => {
  it('keeps every parameter whose empty value means "unset"', () => {
    for (const key of EMPTY_IS_MEANINGFUL) {
      expect(prune({ [key]: '' })).toEqual({ [key]: '' })
    }
  })

  it('drops every other empty parameter, so the URL never carries `?task=`', () => {
    for (const key of ['task', 'project', 'q', 'status', 'priority', 'type', 'title']) {
      expect(prune({ [key]: '' })).toEqual({})
    }
  })

  it('keeps non-empty values untouched', () => {
    expect(prune({ project: 'p1', team: 't1', q: 'crash' })).toEqual({
      project: 'p1',
      team: 't1',
      q: 'crash',
    })
  })
})
