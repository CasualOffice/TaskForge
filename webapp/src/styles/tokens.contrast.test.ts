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

/**
 * The **design system's** colours, not this product's.
 *
 * `tokens.css` used to declare every hex here; it now aliases
 * `@schnsrw/design-system`, so the values live one package away and a test
 * reading local `var(...)` references would assert nothing while appearing to
 * pass. The pairs below are still TaskForge's — they name the combinations this
 * product actually renders — but the numbers come from the source.
 *
 * That makes this a check on a *dependency*, deliberately: adopting a shared
 * palette does not transfer responsibility for whether this product's screens
 * are legible, and a design system bump that dimmed muted text should fail here
 * rather than ship.
 */
const DESIGN_SYSTEM = resolve(
  HERE,
  '../../node_modules/@schnsrw/design-system/dist/tokens/colors.css',
)
/**
 * Both sources, in cascade order.
 *
 * Most tokens are the suite's. A few are genuinely this product's — the forge
 * orange mark, and any value TaskForge deviates on for a stated reason — and
 * those live in `tokens.css`. Reading the local file *second* mirrors the
 * import order in `app.css`, so what the test measures is what the browser
 * resolves.
 */
const SUITE_CSS = readFileSync(DESIGN_SYSTEM, 'utf8')
const LOCAL_CSS = readFileSync(resolve(HERE, 'tokens.css'), 'utf8')

/**
 * The custom properties of one theme block.
 *
 * `:root` and `:root[data-theme='dark']` are read separately and the dark map
 * is layered over the light one, mirroring the cascade: dark overrides some
 * tokens and inherits the rest, and a pair that reads an inherited token must
 * be checked against the value the browser would actually use.
 */
function block(source: string, selector: string, required = true): Map<string, string> {
  // Anchored to the start of a line, because both files *mention* their own
  // selectors in prose above them — the design system's colours file opens with
  // "dark theme under [data-theme='dark']". A plain `indexOf` matched that
  // comment, parsed from the wrong brace, and produced a map missing every dark
  // value while appearing to succeed: the dark run then measured the mark
  // against the *light* canvas and failed for a reason that was not real.
  const anchored = new RegExp(`^${selector.replace(/[.*+?^${}()|[\]\\]/g, '\\$&')}`, 'm')
  const start = source.search(anchored)
  if (start === -1) {
    // The local file has no dark block worth parsing when it overrides nothing
    // there, and that is not a failure — the suite's dark values stand.
    if (!required) return new Map()
    throw new Error(`no ${selector} block`)
  }
  const open = source.indexOf('{', start)
  const end = source.indexOf('\n}', open)
  const body = source.slice(open + 1, end)
  const out = new Map<string, string>()
  for (const match of body.matchAll(/(--[\w-]+)\s*:\s*([^;]+);/g)) {
    const name = match[1]
    const value = match[2]
    if (name === undefined || value === undefined) continue
    out.set(name, value.trim())
  }
  return out
}

/**
 * The suite first, this product's overrides second — the order `app.css`
 * imports them in, so what is measured is what the browser resolves. Parsed per
 * file rather than from a concatenation: `indexOf(':root {')` finds only the
 * first one, so a merged string would silently discard every local override.
 */
const LIGHT = new Map([...block(SUITE_CSS, ':root {'), ...block(LOCAL_CSS, ':root {')])
// The design system scopes dark to `[data-theme='dark']` without a `:root`
// prefix, so it applies to any element carrying the attribute. Matched exactly
// rather than loosely: a substring search for `data-theme` would also hit the
// file's own comment about it.
/**
 * Follow a `var(--x)` alias to the value it names.
 *
 * TaskForge's tokens are now mostly aliases onto the suite's, so a raw lookup
 * returns the literal string `var(--color-text)` and every ratio would be
 * measured against nonsense. Following the reference is what the browser does,
 * and it is what makes an alias testable at all.
 *
 * Bounded, because a token that referred to itself would otherwise hang the
 * suite rather than fail it.
 */
/**
 * Composite a translucent token over the ground it is drawn on.
 *
 * The suite's dark tints are `rgba(96, 165, 250, 0.16)` — a wash over whatever
 * is behind them, which is how a tint should be built. A ratio computed against
 * the literal string is not a ratio at all, and five pairs "failed" for that
 * reason alone rather than because anything was illegible.
 *
 * Compositing is what the browser does, so it is what the gate must do.
 */
function flatten(value: string, under: string): string {
  const rgba = /^rgba?\(\s*(\d+)\s*,\s*(\d+)\s*,\s*(\d+)\s*(?:,\s*([\d.]+)\s*)?\)$/.exec(value)
  if (rgba === null) return value
  const alpha = rgba[4] === undefined ? 1 : Number(rgba[4])
  const back = /^#([0-9a-f]{2})([0-9a-f]{2})([0-9a-f]{2})$/i.exec(under)
  if (back === null) return value
  const mix = (channel: number, behind: number): number =>
    Math.round(channel * alpha + behind * (1 - alpha))
  const parts = [1, 2, 3].map((index) =>
    mix(Number(rgba[index]), parseInt(back[index] as string, 16)),
  )
  return `#${parts.map((c) => c.toString(16).padStart(2, '0')).join('')}`
}

function resolve_alias(tokens: Map<string, string>, name: string): string | undefined {
  let value = tokens.get(name)
  for (let hop = 0; hop < 8; hop += 1) {
    if (value === undefined) return undefined
    const alias = /^var\(\s*(--[\w-]+)\s*\)$/.exec(value)
    if (alias === null) return value
    value = tokens.get(alias[1] as string)
  }
  return undefined
}

const DARK = new Map([
  ...LIGHT,
  ...block(SUITE_CSS, "[data-theme='dark']"),
  ...block(LOCAL_CSS, ":root[data-theme='dark']", false),
])

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
  const toLinear = (c: number): number => (c <= 0.04045 ? c / 12.92 : ((c + 0.055) / 1.055) ** 2.4)
  const channel = (i: number): number => toLinear(Number.parseInt(full.slice(i, i + 2), 16) / 255)
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
/**
 * The suite's name for each token this product used to own.
 *
 * Written out rather than inferred by stripping a prefix: `--tf-surface-subtle`
 * is the suite's `--color-surface-alt`, and `--tf-accent-fg` is
 * `--color-accent-fg`. A rule that guessed would silently test the wrong pair.
 */
const SUITE: Readonly<Record<string, string>> = {
  '--tf-bg': '--color-bg',
  '--tf-surface': '--color-surface',
  '--tf-surface-subtle': '--color-surface-alt',
  '--tf-surface-sunken': '--color-surface-strip',
  '--tf-text': '--color-text',
  '--tf-text-secondary': '--color-text-secondary',
  '--tf-text-muted': '--color-text-muted',
  '--tf-border-strong': '--color-border-strong',
  '--tf-focus': '--color-focus-ring',
  '--tf-info': '--color-info',
  '--tf-info-subtle': '--color-info-soft',
  '--tf-success': '--color-success',
  '--tf-success-subtle': '--color-success-soft',
  '--tf-warning': '--color-warning',
  '--tf-warning-subtle': '--color-warning-soft',
  '--tf-danger': '--color-danger',
  '--tf-danger-subtle': '--color-danger-soft',
  '--tf-accent': '--color-accent',
  '--tf-accent-fg': '--color-accent-fg',
  '--tf-accent-subtle': '--color-accent-soft',
}

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

  // The border that identifies a control (WCAG 1.4.11). These two lived in
  // KNOWN_CONFLICTS because §5 fixed `--tf-border-strong` at #d4d4d8, which is
  // 1.48:1 on white — the foundation contradicting itself. The redesign amended
  // the token, which is what that block said would move them here.
  ['--tf-border-strong', '--tf-bg', 3, 'control border on the canvas'],
  ['--tf-border-strong', '--tf-surface-subtle', 3, 'control border on a subtle surface'],

  // The interactive accent, as a filled control's ground under white ink, and
  // as a boundary. This is the pair the orange accent could never clear.
  ['--tf-accent-fg', '--tf-accent', 4.5, 'a primary control label on its ground'],
  ['--tf-accent', '--tf-bg', 3, 'the accent as a boundary on the canvas'],
  ['--tf-accent', '--tf-accent-subtle', 4.5, 'accent text on its tint'],

  // The third ground the redesign introduced carries the same text as the
  // other two, so it is held to the same thresholds.
  ['--tf-text', '--tf-surface-sunken', 4.5, 'body text on the sunken surface'],
  ['--tf-text-secondary', '--tf-surface-sunken', 4.5, 'secondary text on the sunken surface'],
  ['--tf-text-muted', '--tf-surface-sunken', 4.5, 'muted text on the sunken surface'],

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
const KNOWN_CONFLICTS: readonly Pair[] = []

describe.each([
  ['light', LIGHT],
  ['dark', DARK],
])('%s theme clears WCAG AA', (_theme, tokens) => {
  it.each(PAIRS)('%s on %s >= %s:1 — %s', (fg, bg, min, _why) => {
    // The canvas every translucent token is ultimately drawn over.
    const canvas = resolve_alias(tokens, '--color-bg') ?? '#ffffff'
    const rawForeground = resolve_alias(tokens, fg) ?? resolve_alias(tokens, SUITE[fg] ?? fg)
    const rawBackground = resolve_alias(tokens, bg) ?? resolve_alias(tokens, SUITE[bg] ?? bg)
    const background = rawBackground === undefined ? undefined : flatten(rawBackground, canvas)
    const foreground =
      rawForeground === undefined ? undefined : flatten(rawForeground, background ?? canvas)
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
      const ratio = contrast(
        (resolve_alias(tokens, fg) ?? resolve_alias(tokens, SUITE[fg] ?? fg)) as string,
        (resolve_alias(tokens, bg) ?? resolve_alias(tokens, SUITE[bg] ?? bg)) as string,
      )
      expect(
        Number(ratio.toFixed(2)),
        `${fg} now clears ${min}:1 — move this pair into PAIRS`,
      ).toBeLessThan(min)
    },
  )
})
