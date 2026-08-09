# 54 — Premium Webapp Design System

## Outcome

TaskForge has a calm, polished work surface that remains fast at tracker density.
The primary canvas is white, hierarchy is clear without ornamental chrome, and
advanced capability is progressively disclosed. Empty states explain the state
and offer the next permitted action. A workspace may configure one accessible
accent colour; blue is the default. The TaskForge orange mark, focus colour, and
semantic colours remain product-owned.

This note is final. The product direction and the configurable-blue default were
approved on 2026-08-09.

## Research (sources + dates checked)

Checked 2026-08-09:

- [WCAG 2.2](https://www.w3.org/TR/WCAG22/) fixes the AA contrast, reflow,
  keyboard, focus, and target requirements used below.
- [W3C target size guidance](https://www.w3.org/WAI/WCAG22/Understanding/target-size-minimum)
  specifies a 24 by 24 CSS pixel AA target or sufficient spacing under its
  listed exceptions.
- [W3C reflow guidance](https://www.w3.org/WAI/WCAG22/Understanding/reflow.html)
  requires content to work at 320 CSS pixels without two-dimensional scrolling,
  except where the content genuinely needs it.
- [Atlassian colour foundations](https://atlassian.design/foundations/color-new/)
  separate brand, semantic, accent, and interaction-state roles.
- [Atlassian empty-state guidance](https://atlassian.design/foundations/content/designing-messages/empty-state)
  uses an informative title, a reason or next step, and an imperative action.
- [Linear's UI redesign](https://linear.app/now/how-we-redesigned-the-linear-ui)
  shows that perceived quality comes from coherent tokens, restrained density,
  and predictable interaction rather than decorative volume.

Research is evidence for the rules, not permission to copy another product's
assets, text, or source.

## Design

### Domain / schema impact

A workspace has one optional appearance value:

```json
{ "appearance": { "primary_color": "#2563EB" } }
```

It is stored under the existing `workspace.settings` JSONB column, whose policy
already permits sparse, admin-only settings that are never queried by field.
There is no new table, user-facing noun, filter, index, or migration. Absence
means `#2563EB`.

The API accepts only canonical `#RRGGBB`. The colour must reach 4.5:1 against
white so white text on a primary action remains AA. The server normalizes the
value to uppercase and rejects a colour outside the contract with
`400 TF-VAL-0004`; the response details name `appearance.primary_color` and the
minimum ratio. The losing side is explicit: light brand colours such as yellow
cannot be the action colour. An administrator may choose a darker expression of
that brand instead.

### Token roles

The shipped stylesheet is the source of truth. Compatibility aliases are
removed after consumers move to the canonical names.

| Role | Default | May workspace configure it? |
| --- | --- | --- |
| Canvas / surface | `#FFFFFF` | No |
| Primary / selected | `#2563EB` | Yes, subject to contrast validation |
| TaskForge mark | orange asset | No |
| Focus ring | `#1D4ED8` | No |
| Success / warning / danger | semantic tokens | No |
| Text / border / elevation | neutral tokens | No |

Primary hover and pressed tokens are deterministic darker states derived by the
client from the accepted colour. The configured colour is never reused for
warnings, focus, links inside prose, or status meaning. Status always carries a
text or icon cue in addition to colour.

### Geometry, type, and icons

All dimensions use a 4 px base grid.

| Item | Compact | Default | Prominent |
| --- | ---: | ---: | ---: |
| Control height | 28 px | 32 px | 36 px |
| Pointer target | 32 px | 36 px | 40 px |
| Touch/coarse target | 44 px | 44 px | 48 px |
| Icon glyph | 16 px | 18 px | 20 px |

The 24 px WCAG target is the hard floor. TaskForge uses the larger values above
where space permits and switches to at least 44 px under coarse input.

| Text role | Size / line height | Weight |
| --- | --- | ---: |
| Page title | 30 / 36 px | 600 |
| Surface title | 22 / 28 px | 600 |
| Section title | 16 / 22 px | 600 |
| Task title / body | 14 / 20–21 px | 400–600 |
| Control | 13 / 18 px | 500 |
| Metadata | 12 / 16 px | 500 |
| Key / code / count | 12 / 16 px | 500 mono, tabular |

One SVG icon language is used: 1.5–1.75 px strokes, round joins and caps, a
20 px view box, and `currentColor`. A glyph is decorative inside a labelled
control and hidden from assistive technology. An icon-only control has an
accessible name and a tooltip; the target is larger than the glyph.

### Layout and progressive disclosure

The shell has three stable regions: a compact product rail, a quiet top bar,
and one work surface. The top bar contains workspace context, global search,
help, appearance, and account actions. Product work does not compete with
administration in the permanent navigation.

The work toolbar shows, in order: project or view context, search, Filter with
active count, Group, Sort, and the primary creation action. Less-used filters
live in one menu; active filters remain visible as removable chips. On narrow
screens, labels collapse after their meaning has been learned from the visible
heading and accessible name, and secondary controls move into an overflow menu.

Breakpoints are content decisions rather than device names:

| Width | Behaviour |
| --- | --- |
| `< 768 px` | bottom/compact navigation, one-column detail, 44 px targets |
| `768–1199 px` | compact rail, reduced toolbar labels, drawer uses most width |
| `>= 1200 px` | full rail labels and multi-column work surfaces |
| `>= 1600 px` | content width grows where a list or board benefits; prose stays bounded |

No document-level horizontal scroll is allowed at 320 CSS px. Boards and data
tables may use a named horizontal work region when their two-dimensional
relationship is essential.

### Empty states and illustrations

There are two empty-state tiers:

- **Full:** an original lightweight SVG illustration, title, one short
  explanation, one primary CTA when the actor may act, and at most one secondary
  link.
- **Compact:** a 24–32 px contextual glyph, title, and optional inline action
  for small panels and filtered groups.

Full illustrations use a 160 by 120 view box and render at 120–180 px on wide
screens or 96–140 px on narrow screens. They contain no essential text, use
tokens, are `aria-hidden`, and remain legible in forced-colour mode. Each SVG is
at most 12 KiB raw; all illustrations in the initial shell total at most 8 KiB
gzip. Illustrations appear for a true blank slate, no results, a cleared inbox,
or a refused/offline recovery state. They do not fill incidental whitespace.

CTA text starts with a verb: `Create task`, `Clear filters`, `Retry`, or
`Choose workspace`. A forbidden action is absent; the empty state may explain
who can grant access but never renders a control the server would refuse.

### Motion and perceived quality

Motion communicates continuity: 90 ms for hover/focus, 140 ms for menus, and
200 ms for drawers/dialogs. Transform and opacity are preferred. Reduced-motion
users receive immediate state changes. Skeletons preserve geometry and do not
pulse indefinitely. Shadows are reserved for overlays; borders, spacing, and
type establish hierarchy on the white canvas.

### Layers & files touched

- `webapp/src/styles/`: canonical tokens, primitives, shell, work surfaces,
  overlays, empty states, and responsive rules, each split by reason to change
  and below the repository's approximate 500-line limit.
- `webapp/src/shell/`: shell, responsive navigation, appearance application,
  session states, and empty-state composition.
- `webapp/src/views/`: progressively disclosed toolbars and contextual states.
- `webapp/public/illustrations/`: original bounded SVG assets.
- `crates/casual-task-api` and `casual-task-persistence`: validated appearance
  read/write through the workspace aggregate.

### API surface

`WorkspaceBody` gains `version` and `appearance.primary_color`. The list and
read endpoints return the same representation.

`PATCH /api/v1/workspaces/{workspace_id}` accepts either or both fields:

```json
{
  "name": "Platform",
  "appearance": { "primary_color": "#1D4ED8" }
}
```

An empty patch is `400 TF-VAL-0003`. `If-Match` remains required. The mutation
requires `workspace.manage`, updates the aggregate version, and writes activity,
audit, and outbox records in the same transaction. Raw `settings` JSON is never
exposed. Existing rename clients remain valid.

The mutation emits `workspace.updated` at schema version 1. Its payload contains
`workspace_id` and `changed_fields` (`name` and/or
`appearance.primary_color`); activity and audit carry the before/after display
values. Consumers use `changed_fields` to invalidate the workspace
representation and never infer authority from the event.

### Failure modes & limits

- Invalid or low-contrast colour: reject the whole patch; return the field and
  measured ratio.
- Unknown appearance key: `400 TF-VAL-0002` through `deny_unknown_fields`.
- Stale version: existing `409` contract; no partial appearance write.
- Missing authority: `403`; tenant invisibility remains `404` at the extractor.
- Missing stored value: render the blue default, not an unstyled frame.
- Malformed stored JSON from an older build: log no customer content, use the
  default, and expose the defect to operators through a bounded metric.

### Security & tenancy implications

Only a member whose resolved authority includes `workspace.manage` may change
appearance. Persistence takes `Scoped`; the workspace id is never accepted as a
repository argument. Appearance is inert data: it is parsed as six hex digits
and assigned to CSS custom properties, never inserted as CSS text or markup.

## Alternatives considered

- **Orange as the configurable primary.** Rejected because the mark and actions
  would become one visual signal, and some orange values miss white-text
  contrast.
- **An unrestricted theme editor.** Rejected because every extra colour creates
  contrast combinations the product cannot gate exhaustively and makes support
  screenshots harder to interpret.
- **Per-user themes first.** Rejected for this increment: workspace identity is
  the approved requirement. The API shape can later add a user preference with
  a clear precedence rule.
- **Generated multi-shade themes from arbitrary colours.** Rejected for this
  increment because perceptual colour generation adds a dependency and a much
  larger verification surface. One validated colour and deterministic pressed
  states meet the current need.

## Acceptance gates

1. Token tests verify every shipped text, focus, semantic, and primary pair in
   light presentation at its WCAG threshold; no known-conflict allow-list.
2. Component tests verify full and compact empty states, CTA authority, accessible
   names, reduced motion, and forced-colour behavior.
3. Playwright covers 320, 768, 1200, and 1600 CSS px, 200% zoom/reflow, keyboard
   order, visible focus, coarse targets, and screenshot baselines.
4. API tests cover canonicalization, contrast rejection, unknown keys, empty
   patch, authorization, tenant isolation, stale `If-Match`, and atomic history.
5. The initial shell remains at or below 200 KiB gzip and the illustration
   sub-budget is reported separately.
6. Every CSS and TypeScript module remains below the repository size limit and
   lint forbids new legacy token references.

## ADRs triggered

- **ADR-033** — one validated workspace accent in existing settings; public API
  and security-bound validation.

## Tracker IDs

- **D-066** — premium webapp design system and configurable appearance.
- **C-025** — design-system foundation, responsive shell, and empty states.
- **C-026** — workspace appearance API and persistence.
