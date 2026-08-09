# TaskForge Layout & Interaction Guidelines

**Status:** Proposed baseline  
**Goal:** Give TaskForge a recognizable, flexible workspace model without turning it into a normalized Jira/Linear clone.

## 1. Layout model

TaskForge has four layout layers:

1. **Global layer** — workspace switching, global search/command access, create, notifications, account.
2. **Product rail** — small set of permanent destinations.
3. **Context layer** — optional project/view-specific navigation and filters.
4. **Work surface** — the actual list, board, activity, settings or administration surface.

The context layer is optional. Never render an empty sidebar merely to preserve symmetry.

### Common desktop compositions

```text
A. Focused
┌──────┬────────────────────────────────────────────┐
│ Rail │ Work surface                               │
└──────┴────────────────────────────────────────────┘

B. Contextual
┌──────┬────────────────┬───────────────────────────┐
│ Rail │ Context        │ Work surface              │
└──────┴────────────────┴───────────────────────────┘

C. Detail
┌──────┬───────────────┬─────────────────┬──────────┐
│ Rail │ Context       │ Work surface    │ Drawer   │
└──────┴───────────────┴─────────────────┴──────────┘
```

These are compositions, not fixed page templates.

## 2. Desktop geometry

Recommended starting ranges:

- Global bar: 48–52 px.
- Product rail: 52–60 px.
- Context navigation: 200–240 px.
- Task detail drawer: 420–560 px, responsive to viewport.
- Main work surface: fluid; owns remaining width.
- Content padding: 16–24 px depending on density.
- Do not cap boards/tables to a marketing-style centered max-width.

## 3. Navigation

Permanent navigation should remain small:

- My Work
- Projects
- Search
- Activity/Inbox

Administration and configuration are contextual/secondary.

`Cmd/Ctrl+K` is a primary interaction surface for create, jump, assign, transition, search and extension commands. Command palette capability must not justify hiding essential discoverable actions.

## 4. Work surfaces

### Task list

The list is the highest-density canonical view.

- Row title is visually dominant.
- Keep identifier, state, priority, assignee and selected metadata aligned.
- Columns can be user-configurable, but defaults stay restrained.
- Row actions appear on hover/focus without moving content.
- Virtualize long lists.
- Selection must support keyboard and range operations.

### Board

Columns may differ in width when content demands it; equal columns are not a rule.

Task cards show by default:

1. task title;
2. task identifier;
3. at most 2–3 high-value signals.

Do not repeat status when the column already communicates it.

### Task detail

**A full route is the default, not the drawer.** The drawer was tried first and
failed: a 420–560 px column cannot show what a task *is*, who owns it, what it
blocks, and what people have said about it without scrolling, and everything
below the fold is effectively invisible. A reader had to scroll to learn the
assignee.

**Nothing essential may sit below the fold.** On a normal desktop viewport a
task's identity, state, people, dates, relationships and the most recent
conversation are all visible at once. Scrolling is for the tail of a long
thread or a long description — never for a field.

That means **columns, not a single stack**:

```text
┌────────────────────────────┬───────────────────┐
│ Identity, description      │ Status, assignees │
│ Subtasks, blockers         │ Priority, dates   │
│ Comments (most recent      │ Project, reporter │
│ first, tail scrolls)       │ Tags, milestone   │
└────────────────────────────┴───────────────────┘
```

The right column is metadata and is short by construction; the left column
carries the narrative. Only the comment thread scrolls, and it scrolls within
itself rather than moving the page.

Use the drawer only for a **peek** — a quick look from a board or list that
preserves position, showing identity, state, assignee and description, with an
obvious route to the full view. It is not a smaller version of the detail page;
it is a different, deliberately partial thing.

The full route is also what direct links, new tabs and narrow screens get.

Both surfaces share one information architecture and one component set, so a
field cannot exist in one and be forgotten in the other.

## 5. Toolbar behavior

A toolbar is contextual, not decorative.

Preferred ordering:

```text
[View identity]   [Filter] [Group] [Sort]             [Create]
```

Do not fill every toolbar with text buttons. Use icon-only actions only when the symbol is conventional and the control has a tooltip/accessibility label.

Toolbars should not exceed two rows. If they do, move secondary actions into menus or the command palette.

## 6. Forms

- Labels remain visible; placeholder text is not a label.
- Show required fields first.
- Advanced/custom/plugin fields use progressive disclosure.
- Validation occurs near the affected field.
- Destructive actions are spatially separated from routine save actions.
- Avoid modal forms for workflows that require referencing the underlying task/project.

## 7. Responsive behavior

### ≥ 1200 px
Full rail + optional context + work surface + optional drawer.

### 768–1199 px
Rail remains; context may collapse into a temporary panel. Drawer can occupy 45–60% width.

### < 768 px
One primary surface at a time. Navigation becomes temporary. Task detail becomes a full surface. Touch targets use the comfortable/touch tier.

Responsive behavior must preserve capability, not merely hide controls.

## 8. State hierarchy

A screen should visually answer, in this order:

1. Where am I?
2. What work is here?
3. What needs attention?
4. What can I do next?
5. Where are advanced controls?

If metadata answers none of those questions, it should probably be hidden by default.

## 9. Overlays

Use:

- Tooltip — explanation only.
- Popover — small contextual controls.
- Menu — actions/choices.
- Drawer — substantial detail while preserving context.
- Dialog — blocking decision or short focused operation.
- Command palette — navigation and command execution.

Do not use a modal where a drawer or inline interaction preserves useful context.

## 10. Empty states

Empty states are operational.

Example:

> No tasks match this view. Change the filters or create a task.

Avoid decorative illustrations by default and avoid motivational marketing copy inside authenticated work surfaces.

## 11. Interaction performance

Perceived speed is part of layout quality.

- Local response to pointer/keyboard input should feel immediate.
- Avoid layout shifts during mutation.
- Preserve scroll/board position when a drawer closes.
- Cache view state intentionally.
- Optimistic state must reconcile with server authority.
- SSE/live updates should patch affected surfaces rather than reload entire views.

## 12. Relationships: blockers and subtasks

Relationships are the one place where metadata changes what a user is *allowed*
to do, so they are presented as consequences, not as decoration. The semantics
are fixed in [`docs/03-DOMAIN-MODEL.md`](../docs/03-DOMAIN-MODEL.md); this
section says only how they appear.

### Blockers

`BLOCKS` is the only dependency kind in v1. "Relates" and "duplicates" are
presentational links and are deferred — do not design placeholders for them.

**Two lists, never one.** A task's drawer shows *Blocked by* and *Blocking* as
separate lists. Merging them into "Dependencies" makes the reader work out the
direction from the arrow, and the direction is the entire meaning.

Each entry shows identifier, title, and the blocker's state. An unresolved
blocker is visually distinct from a resolved one; do not communicate that with
colour alone (§7 of the foundation).

**A task the viewer cannot see renders as `Restricted`** with its identifier
withheld — never as its title, never as a broken row, never as an error. It is a
normal state of the list, not a failure of it.

### Blocked work is refused before the gesture, not after

Dependencies gate transitions. A blocked task must not accept a drag into an
`ACTIVE` or `COMPLETED` column and then spring back on a refusal — a card that
appears to move and then does not reads as a bug.

- The card carries a blocked indicator, so the state is visible before anyone
  reaches for it.
- The drop target is disabled for that card, with the reason available on
  hover/focus and in the keyboard-move announcement.
- The transition control in the drawer is disabled for the same reason, and
  names the blocking task.

`BlockedNotice` is the shared explanation: one line naming what blocks this, and
a link to it when the viewer may see it.

### Overriding a blocker

Two routes exist and they are not equivalent:

- The workflow's transition sets `ignore_dependencies` — no user decision, no
  prompt, nothing to design.
- The actor holds `task.dependency.override` — a deliberate act that **records
  an audit event with a reason**.

The second requires `OverrideDialog`: a blocking decision (§9 permits a dialog
here), stating what is blocked, by what, and requiring the reason as text before
the confirm enables. The reason is stored and shown in the activity timeline. A
free override with no reason is an audit row nobody can interpret later.

Users without the permission see the refusal explained, not the override
control.

### Subtasks

Depth is capped at one, permanently. There is **no tree view**, no expand
chevron on a subtask, and no indentation beyond one level. Deeper decomposition
uses dependencies or milestones.

- A parent shows a `SubtaskList` with a rollup — `3/5 done`.
- **The rollup is displayed, never enforced.** Nothing in the interface may
  suggest that completing children completes the parent, and no control offers
  to do it. Implicit status change is the most confusing behaviour in every
  tracker that does it.
- A subtask shows its parent as a single line of context near the identifier,
  not as a breadcrumb trail — the trail can only ever be one level deep.
- A subtask is a full task: its own identifier, status, assignees and
  permissions. It is never rendered as a checklist item.

### Where relationships appear

| Surface | Shows |
|---|---|
| Board card | blocked indicator only, and only when blocked; subtask rollup on a parent |
| List row | blocked indicator; parent identifier when the row is a subtask |
| Drawer | `Blocked by`, `Blocking`, `Subtasks`, each with its own add control |
| Activity | every relation added or removed, and every override with its reason |

A card already showing status, priority and assignee is at the §4 limit of two
to three signals. **Blocked replaces a signal; it does not add a fourth.**

### Creating a relation

Adding a blocker is a search-and-pick over tasks the viewer may see, in a
popover, not a modal — the user needs the current task visible while choosing.

**A cycle is refused at write time** and the message names the path that would
close the loop (`ONB-4 → API-2 → ONB-4`). "Invalid dependency" tells the user
nothing about what to do next.
