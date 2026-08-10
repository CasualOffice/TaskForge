/**
 * Which composition is in force.
 *
 * `docs/46` §8: the narrow task page is a *separate composition*, not the
 * desktop one reordered — metadata moves behind a "More details" disclosure
 * rather than being stacked above the title. A disclosure is an element, not a
 * style, so CSS alone cannot express it and the layout has to know its own
 * width.
 *
 * `matchMedia` and not a resize listener: the browser already computes this and
 * a listener would recompute it on every frame of a drag.
 */
import { useEffect, useState } from 'react'

/** The breakpoint `docs/46` §3 names, in one place. */
export const NARROW = '(max-width: 1024px)'

/**
 * `window.matchMedia`, when the environment has it.
 *
 * jsdom does not, and neither does a server renderer. Guarding on `window`
 * alone is the mistake — the object exists there, the method does not — and the
 * component that asked crashed the whole route rather than losing a media
 * query. Absent, the answer is the wide composition: it is the one with a
 * separate metadata rail, so nothing goes missing when the query cannot be
 * asked.
 */
function query_matches(query: string): boolean {
  return typeof window !== 'undefined' && typeof window.matchMedia === 'function'
    ? window.matchMedia(query).matches
    : false
}

export function useNarrow(query: string = NARROW): boolean {
  const [matches, setMatches] = useState(() => query_matches(query))
  useEffect(() => {
    if (typeof window === 'undefined' || typeof window.matchMedia !== 'function') return
    const list = window.matchMedia(query)
    const onChange = (event: MediaQueryListEvent): void => setMatches(event.matches)
    setMatches(list.matches)
    list.addEventListener('change', onChange)
    return () => list.removeEventListener('change', onChange)
  }, [query])
  return matches
}
