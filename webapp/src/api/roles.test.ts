/**
 * The client's permission list against the server's.
 *
 * # Why this test reads Rust
 *
 * `PERMISSION_GROUPS` is the role editor's whole vocabulary. There is no
 * endpoint that lists permissions — `permission(key)` is a table the server
 * validates against — so the list is typed out, and a typed-out copy of someone
 * else's list drifts. The two failure modes are both silent in the UI:
 *
 * - a key the server has and the client omits is **an authority no admin can
 *   ever grant**, invisible because nothing renders it;
 * - a key the client invents is offered, checked, saved, and refused with
 *   `TF-VAL-0005` — a validation error for a control the product itself drew.
 *
 * Neither shows up in a type check or a render test. Reading
 * `crates/casual-task-model/src/permission.rs` is the only way to make the copy
 * provably a copy. If the file moves, this fails loudly rather than passing
 * vacuously — which is why the parse asserts it found something.
 */
import { existsSync, readFileSync } from 'node:fs'
import { dirname, resolve } from 'node:path'
import { describe, expect, it } from 'vitest'

import { ALL_PERMISSIONS, PERMISSION_GROUPS, PERMISSION_HELP } from './roles'

/**
 * Found by walking up from the working directory rather than resolved against
 * `import.meta.url`: vitest transforms this module, so its URL is not a `file:`
 * one. Walking also means the test works whether it is run from `webapp/` or
 * from the repository root.
 */
const REGISTRY = (() => {
  const relative = 'crates/casual-task-model/src/permission.rs'
  let at = process.cwd()
  for (;;) {
    const candidate = resolve(at, relative)
    if (existsSync(candidate)) return candidate
    const up = dirname(at)
    if (up === at) throw new Error(`${relative} is not above ${process.cwd()}`)
    at = up
  }
})()

/** The `NAME => "key",` lines inside the `permissions! { … }` macro call. */
function serverPermissions(): readonly string[] {
  const source = readFileSync(REGISTRY, 'utf8')
  const block = /permissions!\s*\{([\s\S]*?)\n\}/.exec(source)
  expect(block, `no permissions! block in ${REGISTRY}`).not.toBeNull()
  const keys = [...(block?.[1] ?? '').matchAll(/=>\s*"([^"]+)"/g)].map((m) => m[1] as string)
  expect(keys.length, 'parsed no permission keys').toBeGreaterThan(10)
  return keys
}

describe('the permission vocabulary', () => {
  it('is exactly the set the server knows', () => {
    expect([...ALL_PERMISSIONS].sort()).toEqual([...serverPermissions()].sort())
  })

  it('names each key once, in one group', () => {
    // A key in two groups renders two checkboxes for one authority, and the
    // second one silently wins whichever way the editor collects them.
    expect(new Set(ALL_PERMISSIONS).size).toBe(ALL_PERMISSIONS.length)
  })

  it('explains every key it offers', () => {
    // The editor falls back to the raw key, so a missing gloss is not a crash —
    // it is an admin choosing an authority from a name only its author reads.
    const unexplained = ALL_PERMISSIONS.filter((key) => PERMISSION_HELP[key] === undefined)
    expect(unexplained).toEqual([])
  })

  it('has no empty group', () => {
    expect(PERMISSION_GROUPS.filter((group) => group.keys.length === 0)).toEqual([])
  })
})
