/**
 * The one page envelope, and the one rule about cursors.
 *
 * # The failure this module prevents
 *
 * `OFFSET`. `docs/26` and ADR-011 forbid it, and the way a client reintroduces
 * it is not by sending `?offset=` — the server would refuse that — but by
 * *parsing* `next_cursor` and reconstructing a page number from it. `docs/05`
 * says "clients must not parse it", so the type here is a branded opaque string
 * and there is no function in this file that takes one apart.
 */

/** `crates/casual-task-api/src/wire.rs` — `Page`. */
export interface PageInfo {
  /** Opaque. Passed back verbatim or not at all. */
  readonly next_cursor?: string
  readonly has_more: boolean
}

/** `{ "data": [...], "page": { ... } }` — every list endpoint. */
export interface Paged<T> {
  readonly data: readonly T[]
  readonly page: PageInfo
}

/**
 * `docs/05` §Pagination: default 50, hard ceiling 100.
 *
 * Over-limit is refused rather than clamped server-side, so asking for more than
 * this is a `400` rather than a short page — which is why the constant is here
 * and not a number typed at each call site.
 */
export const MAX_LIMIT = 100

/**
 * The cursor for the next page, or `undefined` when there is none.
 *
 * `has_more` and `next_cursor` are checked together on purpose: the server omits
 * the cursor when the page is the last one, and a client that trusted only
 * `has_more` would loop forever on the final page.
 */
export function nextCursor(page: PageInfo | undefined): string | undefined {
  if (page === undefined || !page.has_more) return undefined
  return page.next_cursor
}
