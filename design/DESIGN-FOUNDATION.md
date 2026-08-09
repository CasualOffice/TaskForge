# TaskForge Design Foundation

**Status:** Proposed baseline  
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
8. **Asymmetric where useful.** Do not force all views into equal columns, equal cards or a universal dashboard grid.

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

Prefer the Casual Office shared font stack. Do not add a webfont to the authenticated shell solely for branding.

| Role | Size / line | Weight |
|---|---|---:|
| Page title | 24 / 30 px | 600 |
| Section title | 18 / 24 px | 600 |
| Task/card title | 14 / 20 px | 550–600 |
| Body | 14 / 21 px | 400 |
| Control | 13–14 / 18 px | 500 |
| Metadata | 12 / 16 px | 400 |
| IDs/code | 12–13 / 18 px | 500 mono |

Avoid marketing-scale headings inside the application.

## 5. Color foundation

The UI must work in grayscale before brand color is applied.

```css
:root {
  --tf-bg: #ffffff;
  --tf-surface: #ffffff;
  --tf-surface-subtle: #f7f7f8;

  --tf-text: #18181b;
  --tf-text-secondary: #52525b;
  --tf-text-muted: #71717a;

  --tf-border: #e4e4e7;
  --tf-border-strong: #d4d4d8;

  --tf-focus: #2563eb;

  --tf-info: #2563eb;
  --tf-success: #15803d;
  --tf-warning: #a16207;
  --tf-danger: #b91c1c;
  --tf-extension: #7e22ce;

  --tf-brand: #d8610b;
  --tf-brand-strong: #a94400;
}
```

Brand color and semantic status colors are separate systems. Never use brand orange to mean warning.

## 6. Borders, radius and elevation

Default hierarchy uses borders, not shadows.

| Element | Radius |
|---|---:|
| Badge | 5 px |
| Input/button | 6 px |
| Card/row container | 8 px |
| Popover/menu | 10 px |
| Drawer/dialog | 12 px |

Use shadows only for surfaces that physically overlay another interaction plane: menus, popovers, command palette, dragged objects, drawers and dialogs.

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

`AppRail`, `ContextNav`, `WorkToolbar`, `TaskCard`, `TaskRow`, `TaskDrawer`, `FilterBar`, `CommandPalette`, `PermissionExplanation`, `ActivityTimeline`, `EmptyState`, `RelationList`, `BlockedNotice`, `SubtaskList`, `OverrideDialog`.

Product patterns may compose primitives and understand domain concepts.

The last four exist because relationships are not decoration in this product:
a blocking edge changes what a user is permitted to do, and
[`LAYOUT-AND-INTERACTION-GUIDELINES.md` §12](LAYOUT-AND-INTERACTION-GUIDELINES.md)
specifies how they behave.

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
- navigation entries for every feature or plugin.
