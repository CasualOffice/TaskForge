/**
 * Light or dark, and the fact that "the user has not chosen" is a third answer.
 *
 * # The failure this module prevents
 *
 * Overriding the operating system. A theme stored as a boolean has no way to say
 * "follow the system", so the first time a user clicks the toggle they lose the
 * OS setting permanently — including the automatic switch at dusk. The stored
 * value is therefore a tri-state, and `'system'` is the default.
 *
 * docs/42 §Accessibility requires 4.5:1 contrast verified in **both** themes, so
 * neither is a second-class variant: `tokens.css` declares the same token names
 * under each.
 */

export type ThemeChoice = 'light' | 'dark' | 'system'

const STORAGE_KEY = 'taskforge.theme'

/** What the user chose, or `'system'` when they have not. */
export function storedChoice(): ThemeChoice {
  const raw = localStorage.getItem(STORAGE_KEY)
  if (raw === 'light' || raw === 'dark' || raw === 'system') return raw
  // No stored preference: light, not `system`. See `resolve` — the foundation
  // designs on a white canvas, and inheriting the OS's dark mode would show a
  // first-time user a surface the design was never checked against.
  return 'light'
}

/** The theme actually in effect, resolving `'system'` against the OS. */
/**
 * The theme a choice resolves to.
 *
 * `system` follows the operating system, and an *unset* preference is not
 * `system`: `design/DESIGN-FOUNDATION.md` §1.5 makes white "the primary
 * application canvas", so a first-time visitor on a machine set to dark must
 * still see the canvas the product was designed on. Dark remains a theme a
 * user can choose — `storedChoice()` returns `light` until they do.
 */
export function resolve(choice: ThemeChoice): 'light' | 'dark' {
  if (choice !== 'system') return choice
  return window.matchMedia('(prefers-color-scheme: dark)').matches ? 'dark' : 'light'
}

/**
 * Apply a choice to the document and remember it.
 *
 * `data-theme` on the root element, which is what `tokens.css` selects on. Set
 * on `documentElement` rather than `body` so the page background is painted
 * before React mounts — otherwise a dark-theme user sees a white flash on every
 * load, which is the kind of detail that makes a product feel unfinished.
 */
export function apply(choice: ThemeChoice): void {
  document.documentElement.dataset['theme'] = resolve(choice)
  if (choice === 'system') localStorage.removeItem(STORAGE_KEY)
  else localStorage.setItem(STORAGE_KEY, choice)
}

/** The next choice in the cycle: system → light → dark → system. */
export function nextChoice(current: ThemeChoice): ThemeChoice {
  if (current === 'system') return 'light'
  if (current === 'light') return 'dark'
  return 'system'
}
