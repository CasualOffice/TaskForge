# TaskForge design record

## What is here, and what is authoritative

| Document | Holds |
| --- | --- |
| [`DESIGN-FOUNDATION.md`](DESIGN-FOUNDATION.md) | Principles, tokens, type, colour, material, accessibility, motion, the refusal component. **v2.** |
| [`LAYOUT-AND-INTERACTION-GUIDELINES.md`](LAYOUT-AND-INTERACTION-GUIDELINES.md) | Layout model, geometry, navigation, work surfaces, toolbars, forms, overlays, empty states, relationships |
| [`VISUAL-IDENTITY.md`](VISUAL-IDENTITY.md) | Mark, brand colour, clear space, favicon, voice, the design bet |
| [`FRONTEND-REQUIREMENTS.md`](FRONTEND-REQUIREMENTS.md) | What the client must let a person **do**. Requirements, no layout |
| `taskforge-mark.svg`, `favicon.svg` | The assets |

The rendered screens live in the Claude Design project
`51100e6b-fefa-4ed1-9709-a9e4ff3b0b46` — Design System, Work, Finding,
Administration, Awareness. **The markdown here is the contract; the canvas
documents are the illustration of it.** Where they disagree, the markdown is
what a reviewer holds an implementation to, and the disagreement is a bug in
one of them.

## v1 → v2

v1 transcribed the foundation and stopped: system grey on system white, borders
instead of hierarchy, a cool neutral ramp under an orange mark. It was correct
and characterless.

v2 keeps every rule that was load-bearing — white canvas, the 4 px grid, AA by
construction, no colour-only status, borders before shadows — and rebuilds the
material. Four things changed, and all four were deliberate:

| | v1 | v2 |
| --- | --- | --- |
| Type | "no webfont in the shell" | Instrument Sans + JetBrains Mono |
| Neutrals | Zinc — a cool ramp | Warm graphite, pulled toward the mark's hue |
| Radius | 5 / 6 / 8 / 10 / 12 | 4 / 6 / 8 / 12 / 16 |
| Type scale | page 24/30, section 18/24 | page 30/36, section 16/22 |

The webfont reversal is the substantive one and its cost is written into
§4 rather than left implicit: a third-party runtime dependency, bundle weight,
and an offline failure mode. It is accepted because mono with tabular figures is
what makes a value look checkable, and this product's argument is that values
are checkable.

**On contrast, v2 changes the pairing rather than rescuing a failure.** v1's
semantic inks already passed AA on white — info 5.17:1, success 5.02:1, warning
4.92:1, danger 6.47:1, extension 6.98:1. v2 pairs each ink with its own wash and
darkens it to hold the ratio there, so a badge reads as well as body text does:
info 5.23:1, success 5.96:1, warning 5.37:1, danger 6.51:1, extension 7.63:1.
The gain is that a tinted surface is now a supported way to carry status, not
that a broken palette was repaired.

## Control-boundary contrast amendment

`--tf-border` remains the 1.34:1 decorative hairline. The 2026-08-09 amendment
adds `--tf-border-strong` for control boundaries; it clears 3:1 against canvas
and subtle surfaces in light and dark presentation. The contrast suite now
holds those pairs as blocking tests rather than a known exception.

## Status

The design is accepted. The client foundation, responsive states, and
workspace appearance path are `Building` under C-025 and C-026;
`docs/13-PARITY-CHECKLIST.md` scores what actually exists.
