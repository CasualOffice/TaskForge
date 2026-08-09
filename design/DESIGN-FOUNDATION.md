# TaskForge Design Foundation

**Status:** v2 — accepted. Supersedes the v1 values below where they differ.  
**Scope:** Product UI, shared design primitives, accessibility, density, interaction and layout behavior.  
**Principle:** TaskForge should be dense, fast, calm, technical and predictable. Branding must never be required to understand the interface.

## 1. Product design principles

1. **Information density without visual density.** Show enough information to act, but do not let every property compete for attention.
2. **Progressive disclosure.** Keep primary work visible; reveal secondary fields, advanced controls and administrative detail when requested.
3. **Context preservation.** Prefer drawers, popovers and inline transitions over unnecessary page replacement.
4. **Keyboard first, pointer excellent.** Core workflows must be fast from keyboard without making mouse use awkward.
5. **White canvas, explicit hierarchy.** The primary application canvas is white. Hierarchy comes from typography, spacing, borders and structure—not washed-out surfaces.
6. **Accessible by construction.** WCAG 2.2 AA is a component acceptance criterion, not a release cleanup task.
7. **Stable geometry.** Loading, optimistic updates and live events should not cause avoidable layout shifts.
8. **Scrolling is a cost, not a layout tool.** A surface that scrolls to reveal a field has hidden that field. Use the width available and put what a user needs to *decide* something in one view; reserve scrolling for genuine tails — a long thread, a long list, a long description. Nested scroll regions inside a scrolling page are the worst case and are not permitted.
9. **One accent.** Orange means brand and current position — never warning. At most twice on a screen.
10. **Values look like values.** Identifiers, codes, counts, dates and durations are mono and tabular, always.
11. **Every state carries a glyph or a word.** Colour is never alone.
12. **Asymmetric where useful.** Do not force all views into equal columns, equal cards or a universal dashboard grid.

## 2. Density and sizing

Use a 4 px base spacing grid.

| Token | Value | Typical use |
|---|---:|---|
| `space-1` | 4 px | icon/text micro-gap |
| `space-2` | 8 px | tight control spacing |
| `space-3` | 12 px | control padding |
| `space-4` | 16 px | standard component gap |
| `space-5` | 20 px | compact section gap |
| `space-6` | 24 px | section separation |
| `space-8` | 32 px | major grouping |
| `space-10` | 40 px | page grouping |
| `space-12` | 48 px | large separation |

### Control tiers

| Tier | Height | Use |
|---|---:|---|
| Compact | 28 px | dense tables, secondary inline actions |
| Default | 32 px | normal form and toolbar controls |
| Comfortable | 36 px | primary actions and prominent toolbar controls |
| Touch | 44 px minimum | touch-oriented surfaces |

Do not make the icon glyph the size of its interaction target.

## 3. Iconography

Use one icon family throughout the product. Material Symbols is acceptable if it remains the repository standard.

| Context | Glyph |
|---|---:|
| Dense metadata/action | 16×16 px |
| Default action | 18×18 px |
| Navigation | 20×20 px |
| Empty state / large contextual | 24–32 px |

Recommended icon-button targets: 28–32 px compact, 32 px default, 36 px prominent. Touch surfaces should provide approximately 44 px targets.

Icons require an accessible name when they are the only visible content of a control. Decorative icons are hidden from assistive technology.

## 4. Typography

**Instrument Sans** for text, **JetBrains Mono** for values.

v1 said "prefer the Casual Office shared font stack; do not add a webfont to the
authenticated shell **solely for branding**". Two faces are loaded here and the
reason is not branding: Instrument Sans is narrow enough for a dense list and has
a real 500 and 600, and mono carries every identifier, code, count, duration and
date. The product's whole argument is that values are checkable, and mono with
tabular figures is how a value looks checkable. A tracker is read in columns;
identifiers that jitter between rows cost a scan on every read.

The cost is accepted knowingly and is a real one: a third-party runtime
dependency, bundle weight, and an offline failure mode. Both faces must have a
system fallback in the stack so a failed font load degrades rather than breaks.

| Role | Size / line | Weight | Face |
|---|---|---:|---|
| Page title | 30 / 36 px | 600 | Sans |
| Surface title — a task, a project, a role | 22 / 28 px | 600 | Sans |
| Section title | 16 / 22 px | 600 | Sans |
| Row and card title | 14 / 20 px | 600 | Sans |
| Body | 14 / 21 px | 400 | Sans |
| Control label | 13 / 18 px | 500 | Sans |
| Metadata | 12 / 16 px | 500 | Sans |
| Eyebrow / column head | 11 / 14 px | 600 | Mono, `.1em` tracking, uppercase |
| Identifiers, codes, counts, dates, durations | 12 / 16 px | 500 | Mono, **tabular** |

Avoid marketing-scale headings inside the application.

## 5. Color foundation

Warm graphite, one accent, calm semantics. The canvas is still white; it is
white that belongs to this product. v1 used a cool ramp under an orange mark,
which is why the product read as a template with a logo dropped into it.

The UI must work in grayscale before brand color is applied.

```css
:root {
  --tf-bg: #ffffff;          /* canvas  */
  --tf-surface: #fbfaf8;     /* paper   */
  --tf-surface-subtle: #f4f2ee; /* sunken */

  --tf-text: #18181b;        /* graphite */
  --tf-text-secondary: #57534e;
  --tf-text-muted: #6e6862;

  --tf-border: #e2ded6;

  --tf-focus: #2c5ee8;

  /* Semantic — an ink over its own wash. Every pair passes AA as text. */
  --tf-info: #1f5fd0;        --tf-info-wash: #edf3fe;
  --tf-success: #146a3c;     --tf-success-wash: #ebf5ee;
  --tf-warning: #8a5a08;     --tf-warning-wash: #fbf3e4;
  --tf-danger: #a81f1b;      --tf-danger-wash: #fcefee;
  --tf-extension: #6b21a8;   --tf-extension-wash: #f4edfc;

  --tf-brand: #d8610b;       /* a fill, not an ink */
  --tf-brand-strong: #a94400;/* text on brand, and brand as text */
  --tf-brand-wash: #fdf3ea;
  --tf-brand-edge: #f3d9bf;
}
```

**Orange appears at most twice on a screen:** the current rail destination, and
the one primary action. `--tf-brand` is a fill; white text needs
`--tf-brand-strong` or darker behind it.

Brand color and semantic status colors are separate systems. Never use brand
orange to mean warning.

### The open item this version does not close

`--tf-border` is **1.34:1** on the canvas, and §7 requires 3:1 for control
boundaries. That is fine for a decorative hairline and not fine for the edge of
a control. The intent is that a control's boundary is carried by the E1 ring in
§6 rather than by this token — but until a control-boundary token exists that
measures 3:1, the contract in §7 is not satisfiable as written. **Open.**

## 6. Borders, radius and elevation

Default hierarchy uses borders. **Elevation states a plane; radius states a
size.**

| Element | Radius |
|---|---:|
| Badge | 4 px |
| Input / button | 6 px |
| Card / row container | 8 px |
| Popover / menu / dialog | 12 px |
| Panel | 16 px |

Three elevation levels exist, and **a card gets none**:

| Level | Shadow | For |
|---|---|---|
| Flat | hairline only | cards, rows, panels on the canvas |
| E1 · lifted | `0 1px 2px rgba(24,20,15,.06), 0 0 0 1px rgba(24,20,15,.06)` | a row under the pointer, a sticky toolbar |
| E2 · overlay | `0 4px 12px rgba(24,20,15,.08), 0 1px 2px rgba(24,20,15,.06), 0 0 0 1px rgba(24,20,15,.05)` | menus, popovers, the dragged card |
| E3 · blocking | `0 16px 40px rgba(24,20,15,.16), 0 2px 6px rgba(24,20,15,.08), 0 0 0 1px rgba(24,20,15,.05)` | dialog, drawer, command palette |

A shadow is a claim that one surface sits above another interaction plane. It is
never decoration.

## 7. Accessibility contract

Target **WCAG 2.2 AA**.

- Normal text contrast: at least 4.5:1.
- Large text contrast: at least 3:1.
- Non-text UI state/boundaries: at least 3:1 where WCAG requires it.
- Visible `:focus-visible` state on every interactive control.
- Core flows fully keyboard operable.
- Status is never communicated by color alone.
- Respect `prefers-reduced-motion`.
- Logical DOM and tab order must match visual order.
- Drag-and-drop requires a non-drag alternative.
- Errors must identify the field/problem in text.
- Icon-only actions require accessible names.
- Do not disable browser zoom.

## 8. Motion

Motion communicates continuity; it is not decoration.

| Motion | Duration |
|---|---:|
| Hover/focus | 80–100 ms |
| Menu/popover | 120–150 ms |
| Drawer/dialog | 180–220 ms |

Avoid list-entry animation, bouncing springs, animated gradients and long skeleton transitions.

## 9. Component architecture

### Primitives

`Button`, `IconButton`, `Input`, `Textarea`, `Select`, `Checkbox`, `Radio`, `Switch`, `Tooltip`, `Popover`, `Menu`, `Dialog`, `Drawer`, `Badge`, `Avatar`, `Tabs`, `Table`, `VirtualList`.

Primitives know nothing about TaskForge domain objects.

### Product patterns

`AppRail`, `ContextNav`, `WorkToolbar`, `TaskCard`, `TaskRow`, `TaskDetail`, `TaskPeek`, `FilterBar`, `CommandPalette`, `PermissionExplanation`, `ActivityTimeline`, `EmptyState`, `RelationList`, `BlockedNotice`, `SubtaskList`, `OverrideDialog`.

Product patterns may compose primitives and understand domain concepts.

`TaskDetail` is the full route and `TaskPeek` the deliberately partial drawer;
[`LAYOUT-AND-INTERACTION-GUIDELINES.md` §4](LAYOUT-AND-INTERACTION-GUIDELINES.md)
says why the drawer stopped being the default. The last four exist because relationships are not decoration in this product:
a blocking edge changes what a user is permitted to do, and
[`LAYOUT-AND-INTERACTION-GUIDELINES.md` §12](LAYOUT-AND-INTERACTION-GUIDELINES.md)
specifies how they behave.

## 9a. The refusal — the component this product lives or dies on

An authorization model that can only say no forces every "why can't I?" through
a human reading the grant table by hand. `docs/04` calls that the most common
support question in any tracker. So a refusal is a designed component with its
own hierarchy, and **never a toast**.

**Four parts, always:**

1. **What was refused**, as a sentence, in the reader's language.
2. **Why**, naming the specific thing — the blocking task, the missing
   permission, the unresolved dependency — not a category.
3. **The registry code** from `docs/20`.
4. **The request id**, so a user who can quote it gets help in one round trip.

Where an action exists that would resolve it, offer it: *Open API-2*,
*Why can't I do this?* (which calls `POST /permissions/explain`).

Never: "Forbidden.", "Something went wrong.", a raw server message, or a
disappearing notification. A refusal the reader cannot act on is the failure
this component exists to prevent.

The same shape carries **Restricted**: a row the viewer may not see keeps its
place and loses its identifier. It is never rendered as absent — absence claims
that nothing is there, which is a different and false statement.

## 10. Loading and failure behavior

- Keep the application shell mounted.
- Use skeletons for initial content whose geometry is known.
- Use small progress indicators for isolated operations.
- Prefer optimistic mutation where rollback is safe and understandable.
- Never hide a failed optimistic operation; restore state and explain the failure.
- Empty, error, offline and permission-denied states are first-class product states.

## 11. Anti-patterns

Do not introduce:

- giant dashboard cards;
- oversized application headings;
- low-contrast gray-on-gray text;
- tiny navigation icons;
- shadows on every surface;
- equal-width grids as the default layout;
- deeply nested permanent sidebars;
- seven badges on a task card;
- multiple icon libraries;
- arbitrary spacing/radius values;
- color-only status;
- full-screen loaders after shell initialization;
- navigation entries for every feature or plugin;
- a scrollbar where the width to avoid one was available;
- nested scroll regions — a scrolling panel inside a scrolling page;
- essential fields below the fold on a detail surface.
