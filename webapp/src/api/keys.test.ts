/**
 * Query keys, and the one property that is not obvious.
 *
 * # The bug this file exists for
 *
 * `/settings/workflow` and the board read the *same* workflow from the *same*
 * URL, and cached it under the same key — but through different functions:
 * `readWorkflow` returns a `Workflow`, and `readWorkflowForEditing` returns
 * `{ data, version }` because the editor needs the `ETag` to write. One key,
 * two shapes. Whichever query ran last won, so opening settings and then a
 * board crashed the board with `workflow.statuses is not iterable` — the board
 * had been handed the editor's envelope.
 *
 * Neither request was wrong, neither component was wrong, and no type caught
 * it: `useQuery` infers its data type from its own `queryFn`, so both screens
 * type-checked against a cache entry only one of them could be right about.
 *
 * Two rules come out of that, and both are asserted below:
 *
 * 1. Reads that return different shapes get different keys, even for the same
 *    resource behind the same URL.
 * 2. The narrower key stays a **child** of the broader one, so the prefix
 *    invalidation that writes already perform reaches both. A status renamed in
 *    settings has to repaint the board; splitting the keys into siblings would
 *    have fixed the crash by making the board go stale instead.
 */
import { describe, expect, it } from 'vitest'

import { keys } from './keys'

const WS = '019fe000-0000-7000-8000-000000000001'
const WF = '019fe000-0000-7000-8000-000000000002'

/** What `queryClient.invalidateQueries({ queryKey })` matches: a prefix. */
function isPrefixOf(prefix: readonly unknown[], key: readonly unknown[]): boolean {
  return prefix.length <= key.length && prefix.every((part, i) => part === key[i])
}

describe('the workflow keys', () => {
  it('are different, because the two reads return different shapes', () => {
    expect(keys.workflowForEditing(WS, WF)).not.toEqual(keys.workflow(WS, WF))
  })

  it('keep the editor’s entry under the board’s, so one invalidation clears both', () => {
    // The half that is easy to get wrong while fixing the other half: a
    // sibling key would stop the crash and start a staleness bug, where a
    // status renamed in settings leaves the board showing the old column until
    // something else happens to refetch it.
    expect(isPrefixOf(keys.workflow(WS, WF), keys.workflowForEditing(WS, WF))).toBe(true)
  })

  it('keep the usage list under it too', () => {
    expect(isPrefixOf(keys.workflow(WS, WF), [...keys.workflow(WS, WF), 'usage'])).toBe(true)
  })

  it('separate two workspaces and two workflows', () => {
    // The tenant prefix is what makes a workspace switch clear everything.
    expect(keys.workflow(WS, WF)).not.toEqual(keys.workflow('other', WF))
    expect(keys.workflow(WS, WF)).not.toEqual(keys.workflow(WS, 'other'))
    expect(isPrefixOf(keys.workspace(WS), keys.workflow(WS, WF))).toBe(true)
  })
})
