/**
 * The contrast gate.
 *
 * # The failure this module prevents
 *
 * A colour that looks fine to the person who picked it. The product's own
 * backlog carries `WEB-14`, "Dark mode contrast fails on secondary text ...
 * measures 3.9:1", which is what happens when contrast is a review comment
 * instead of a build step: someone adjusts a hex by two digits to make a chip
 * "read better" and nobody re-measures.
 *
 * So the pairs are declared here and computed from `tokens.css` itself. A token
 * edit that drops a pair below its threshold fails `pnpm test`, in both themes,
 * without anyone remembering to look.
 *
 * # Why this parses the CSS instead of importing a TS palette
 *
 * The stylesheet is the thing that ships. A parallel TypeScript copy of the
 * palette would be a second source of truth, and the failure mode is precisely
 * that the two disagree — the test would pass against values the browser never
 * renders.
 *
 * # Thresholds (WCAG 2.2)
 *
 * 4.5:1 for body text, 3:1 for large text (>= 18.66px bold or 24px) and for
 * non-text boundaries that carry meaning (1.4.11). Each pair says which it is.
 */
import { readFileSync } from 'node:fs'
import { dirname, resolve } from 'node:path'
import { fileURLToPath } from 'node:url'
import { describe, expect, it } from 'vitest'

const HERE = dirname(fileURLToPath(import.meta.url))
const CSS = readFileSync(resolve(HERE, 'tokens.css'), 'utf8')

/**
 * The custom properties of one theme block.
 *
 * `:root` and `:root[data-theme='dark']` are read separately and the dark map
 * is layered over the light one, mirroring the cascade: dark overrides some
 * tokens and inherits the rest, and a pair that reads an inherited token must
 * be checked against the value the browser would actually use.
 */
function block(selector: string): Map<string, string> {
  const start = CSS.indexOf(selector)
  if (start === -1) throw new Error(`no ${selector} block in tokens.css`)
  const open = CSS.indexOf('{', start)
  const end = CSS.indexOf('\n}', open)
  const body = CSS.slice(open + 1, end)
  const out = new Map<string, string>()
  for (const match of body.matchAll(/(--[\w-]+)\s*:\s*([^;]+);/g)) {
    const name = match[1]
    const value = match[2]
    if (name === undefined || value === undefined) continue
    out.set(name, value.trim())
  }
  return out
}

const LIGHT = block(':root {')
const DARK = new Map([...LIGHT, ...block(":root[data-theme='dark']")])

/** `#rgb` or `#rrggbb` to linear-light channels. */
function channels(hex: string): [number, number, number] {
  const raw = hex.trim().replace('#', '')
  const full =
    raw.length === 3
      ? raw
          .split('')
          .map((c) => c + c)
          .join('')
      : raw
  if (!/^[0-9a-fA-F]{6}$/.test(full)) throw new Error(`not a hex colour: ${hex}`)
  const toLinear = (c: number): number =>
    c <= 0.04045 ? c / 12.92 : ((c + 0.055) / 1.055) ** 2.4
  const channel = (i: number): number =>
    toLinear(Number.parseInt(full.slice(i, i + 2), 16) / 255)
  return [channel(0), channel(2), channel(4)]
}

function luminance(hex: string): number {
  const [r, g, b] = channels(hex)
  return 0.2126 * r + 0.7152 * g + 0.0722 * b
}

export function contrast(a: string, b: string): number {
  const one = luminance(a)
  const two = luminance(b)
  const hi = Math.max(one, two)
  const lo = Math.min(one, two)
  return (hi + 0.05) / (lo + 0.05)
}

/** A foreground token, a background token, and the ratio the pair must clear. */
type Pair = readonly [fg: string, bg: string, min: number, why: string]

/**
 * §7: normal text 4.5:1, large text and non-text boundaries 3:1.
 *
 * Every pair is one that actually renders. A token combination nobody uses is
 * not worth constraining, and constraining it would eventually block a
 * legitimate palette change for a surface that does not exist.
 */
const PAIRS: readonly Pair[] = [
  // Body and secondary text on each of the three canvases (§5).
  ['--tf-text', '--tf-bg', 4.5, 'body text on the canvas'],
  ['--tf-text', '--tf-surface', 4.5, 'body text on a surface'],
  ['--tf-text', '--tf-surface-subtle', 4.5, 'body text on a subtle surface'],
  ['--tf-text-secondary', '--tf-bg', 4.5, 'secondary text on the canvas'],
  ['--tf-text-secondary', '--tf-surface', 4.5, 'secondary text on a surface'],
  ['--tf-text-secondary', '--tf-surface-subtle', 4.5, 'secondary text on a subtle surface'],
  // §11 names "low-contrast gray-on-gray text" an anti-pattern, and the
  // product's own backlog carries WEB-14 ("measures 3.9:1"). Muted is the
  // smallest text that ships, so it is held to the body threshold rather than
  // excused as decorative.
  ['--tf-text-muted', '--tf-bg', 4.5, 'muted text on the canvas'],
  ['--tf-text-muted', '--tf-surface', 4.5, 'muted text on a surface'],
  ['--tf-text-muted', '--tf-surface-subtle', 4.5, 'muted text on a subtle surface'],

  // Semantic inks on their own tints — the chip and notice pairings.
  ['--tf-info', '--tf-info-subtle', 4.5, 'info text on its tint'],
  ['--tf-success', '--tf-success-subtle', 4.5, 'success text on its tint'],
  ['--tf-warning', '--tf-warning-subtle', 4.5, 'warning text on its tint'],
  ['--tf-danger', '--tf-danger-subtle', 4.5, 'danger text on its tint'],
  ['--tf-extension', '--tf-extension-subtle', 4.5, 'extension text on its tint'],
  ['--tf-brand-strong', '--tf-brand-subtle', 4.5, 'brand text on its tint'],

  // The same inks as text directly on the canvas: an overdue date and a
  // failure sentence are both plain text on white.
  ['--tf-danger', '--tf-surface', 4.5, 'overdue text on a surface'],
  ['--tf-danger', '--tf-bg', 4.5, 'failure text on the canvas'],
  ['--tf-success', '--tf-surface', 4.5, 'success text on a surface'],
  ['--tf-warning', '--tf-surface', 4.5, 'warning text on a surface'],
  ['--tf-info', '--tf-surface', 4.5, 'info text on a surface'],
  ['--tf-extension', '--tf-surface', 4.5, 'extension text on a surface'],

  // §7 requires a visible focus state on every control, and 1.4.11 makes the
  // ring a non-text boundary at 3:1 against whatever it lands on.
  ['--tf-focus', '--tf-bg', 3, 'focus ring on the canvas'],
  ['--tf-focus', '--tf-surface', 3, 'focus ring on a surface'],
  ['--tf-focus', '--tf-surface-subtle', 3, 'focus ring on a subtle surface'],

  // VISUAL-IDENTITY §7 lists the four approved mark placements. Orange on
  // white and orange on graphite are two of them; the mark is a non-text
  // graphic, so 3:1 applies (§7 "non-text UI state/boundaries").
  ['--tf-brand', '--tf-bg', 3, 'brand mark on the canvas'],
  ['--tf-brand', '--tf-surface-subtle', 3, 'brand mark on a subtle surface'],
]

/**
 * Pairs where the foundation contradicts itself, recorded rather than resolved.
 *
 * §5 fixes `--tf-border-strong: #d4d4d8`. §7 requires "at least 3:1" for
 * "non-text UI state/boundaries ... where WCAG requires it", and WCAG 2.2
 * §1.4.11 requires it of the boundary that identifies a control. #d4d4d8 is
 * **1.48:1** on the §5 white canvas and 2.05:1 on the dark canvas. Both cannot
 * hold at once.
 *
 * Neither side is this file's to change: editing the token would silently
 * override a specified value, and dropping the pair would hide the conflict.
 * So the conflict is asserted — the test fails if the ratio ever *clears* the
 * threshold, which is the signal that the foundation was amended and the pair
 * belongs back in `PAIRS`.
 *
 * The mitigation in the meantime is that no control relies on this border
 * alone: inputs and buttons in `app.css` carry a fill, a label and a
 * `:focus-visible` ring at `--tf-focus`, which does clear 3:1. That satisfies
 * 1.4.11 in substance while the token is out of line with it.
 *
 * Raised for a foundation decision; see the report accompanying this branch.
 */
const KNOWN_CONFLICTS: readonly Pair[] = [
  ['--tf-border-strong', '--tf-bg', 3, 'control border on the canvas'],
  ['--tf-border-strong', '--tf-surface-subtle', 3, 'control border on a subtle surface'],
]

describe.each([
  ['light', LIGHT],
  ['dark', DARK],
])('%s theme clears WCAG AA', (_theme, tokens) => {
  it.each(PAIRS)('%s on %s >= %s:1 — %s', (fg, bg, min, _why) => {
    const foreground = tokens.get(fg)
    const background = tokens.get(bg)
    expect(foreground, `${fg} is not declared`).toBeDefined()
    expect(background, `${bg} is not declared`).toBeDefined()
    const ratio = contrast(foreground as string, background as string)
    // Reported to two decimals so a failure names the number to fix.
    expect(Number(ratio.toFixed(2))).toBeGreaterThanOrEqual(min)
  })

  // The other half of the contradiction: if one of these ever clears its
  // threshold, the foundation has been amended and the pair must move into
  // `PAIRS`. Asserting the failure is what stops the conflict being forgotten.
  it.each(KNOWN_CONFLICTS)(
    '%s on %s still conflicts with the §7 %s:1 rule — %s',
    (fg, bg, min, _why) => {
      const ratio = contrast(tokens.get(fg) as string, tokens.get(bg) as string)
      expect(
        Number(ratio.toFixed(2)),
        `${fg} now clears ${min}:1 — move this pair into PAIRS`,
      ).toBeLessThan(min)
    },
  )
})
