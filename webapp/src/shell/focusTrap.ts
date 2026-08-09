/**
 * Keyboard containment for overlays.
 *
 * # The failure this module prevents
 *
 * A keyboard trap, and its opposite. docs/42 §Accessibility asks for "no
 * keyboard traps in drawer or palette" — but the fix people reach for first is
 * to remove containment entirely, which is worse: tabbing out of an open drawer
 * lands the focus ring on a board behind a scrim, invisible, and the user is now
 * operating controls they cannot see.
 *
 * The contract WCAG 2.2 actually asks for is: focus cycles *within* the overlay,
 * `Escape` always leaves, and focus returns to whatever opened it. All three, or
 * none of them work.
 */
import { useEffect, type RefObject } from 'react'

/** Everything focusable, in DOM order. `:not([disabled])` matters — a disabled
 *  control is in the DOM and not in the tab order, and including it produces a
 *  Tab press that appears to do nothing. */
const FOCUSABLE =
  'a[href], button:not([disabled]), input:not([disabled]), select:not([disabled]), textarea:not([disabled]), [tabindex]:not([tabindex="-1"])'

/**
 * Contain focus inside `container` until it unmounts.
 *
 * @param onEscape called for `Escape`. Always wired: an overlay that can only be
 * dismissed by clicking a specific pixel is not keyboard operable.
 */
export function useFocusTrap(
  container: RefObject<HTMLElement | null>,
  onEscape: () => void,
): void {
  useEffect(() => {
    const element = container.current
    if (element === null) return

    // Remembered before the first focus move, so it is the control the user
    // actually left — reading it later would find something inside the overlay.
    const returnTo = document.activeElement as HTMLElement | null

    const first = element.querySelector<HTMLElement>(FOCUSABLE)
    ;(first ?? element).focus()

    function onKeyDown(event: KeyboardEvent): void {
      if (event.key === 'Escape') {
        event.stopPropagation()
        onEscape()
        return
      }
      if (event.key !== 'Tab') return

      const host = container.current
      if (host === null) return
      const focusable = [...host.querySelectorAll<HTMLElement>(FOCUSABLE)]
      const edge = event.shiftKey ? focusable[0] : focusable.at(-1)
      if (edge === undefined) return
      // Only the edges are intercepted. Rewriting focus on every Tab would fight
      // the browser's own order and break anything with a nested tab stop.
      if (document.activeElement === edge) {
        event.preventDefault()
        ;(event.shiftKey ? focusable.at(-1) : focusable[0])?.focus()
      }
    }

    document.addEventListener('keydown', onKeyDown, true)
    return () => {
      document.removeEventListener('keydown', onKeyDown, true)
      // Only if focus is still inside — if the user has already clicked
      // elsewhere, yanking it back is the drawer stealing their cursor.
      if (element.contains(document.activeElement)) returnTo?.focus()
    }
  }, [container, onEscape])
}
