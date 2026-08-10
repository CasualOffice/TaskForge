# 46 — The Task Surface: One Experience, Three Presentations

The task is the only object every role opens every day. It has three
presentations — a **list row**, a **quick side panel**, and a **full page** —
and until now each was assembled separately from whatever fields the API
returned. That is the mechanical cause of the complaint this document answers:
information exists on one surface and disappears or moves unpredictably on
another.

> **The problem is not colour or spacing. It is information architecture and the
> scrolling model.**

The three presentations share components and terminology. They differ only in
*purpose*, and purpose decides what each may carry.

| | Purpose | Answers |
| --- | --- | --- |
| **List row** | Scan and decide | "What needs my attention?" |
| **Side panel** | Quick peek | "What is this, what state, who owns it, does anything need attention?" |
| **Full page** | Understand and work | "What happened, and what do I do about it?" |

The rule that follows: **a row triages, a panel answers, a page transacts.** No
presentation attempts the job of the one to its right.

> **Source note.** The competitive section is written from working knowledge of
> those products, not from fetched documentation. Nothing here quotes anyone's
> private design system.

---

## 1. Competitive reading

**Linear — issue peek.** Identifier, title, then a horizontal band of property
pills, then description. Pills rather than label/value rows is what lets eight
facts occupy one band instead of eight lines. *Taken: the band. Left: the
density, which assumes a user who has learned the icons.*

**Jira — issue view.** Narrative left, details right, grouped under headings,
with People and Dates separated. Jira's own redesign moved status **out** of the
details panel and up beside the title, because it is the field people look for
first. *Taken: grouping by question, and status out of the rail. Left: the
accordions — a field you must open is a field you will not read.*

**GitHub — issue page.** One narrative column, a quiet rail, and **the composer
at the end of the timeline**. Hierarchy is carried by whitespace and one heading
weight, with very few boxes. *Taken: restraint with borders.*

**Asana — task pane.** The pane *is* the page, docked. That only works because
Asana's field set is small; ours is not, which is exactly why our pane read as a
property dump. *Taken: the confirmation that we need two compositions, not one.*

**Height, Shortcut.** Both put a one-sentence status line under the title before
any field list.

**What none of them have: the two clocks.** No mainstream tracker separates
*what state work is in* from *where it has reached*, nor shows custody passing
between teams. That is ours (`docs/45`), so it earns a line on every one of the
three presentations rather than a row in a panel.

---

## 2. Fundamentals

1. **One question per region.** A region answering two questions gets split.
2. **Hierarchy is size, weight, then space.** Colour is a fourth resort and
   never the only signal (WCAG 2.2 §1.4.1) — status and priority always carry
   text.
3. **A heading must outrank its own contents.**
4. **The 8-point grid**, with rhythm *between* regions larger than *within*
   them. Proximity groups before any border does.
5. **Borders are the last grouping tool.** Boxes inside boxes are a smell —
   which is why metadata is a compact definition list and **not a series of
   cards**.
6. **Progressive disclosure** for rare, consequential acts; never for the
   frequent ones. Status is frequent, so it is always one press.
7. **Measure** capped near 70 characters for prose.
8. **44 × 44 CSS px** touch targets on coarse pointers.
9. **The narrow order is the reading order.** Reflow (WCAG 2.2 §1.4.10) is not
   satisfied by stacking the same regions — metadata before the title fails it
   even though it "fits".
10. **No engineering artefacts in product copy.** No decision IDs, endpoint
    names, schema limits, permission keys or tracker references on any of these
    surfaces. They belong in `docs/` and in development diagnostics.

---

## 3. The scrolling model — settled

This is the second half of the complaint, and it is fixed here before any CSS.

> **One scrolling region per route, at every width. Nothing nests.**

| Width | What scrolls | The rail |
| --- | --- | --- |
| ≥ 1024 px | The narrative column. One region. | `position: sticky`, **not** a second scroller |
| < 1024 px | The page. One region. | Becomes the *More details* disclosure at the end |

Consequences, stated so they cannot be re-argued in CSS:

- `.shell__main` may not be `overflow: hidden` on a route whose content scrolls.
- No panel, section, comment thread, board column body or metadata rail may
  introduce its own `overflow: auto` on a route that already scrolls. The board
  is the single exception: its **columns** scroll, because a board is a
  horizontal set of vertical lists and that is what it is.
- A section must never be cut off mid-control by a container it does not know
  about. If a region can overflow, the *route* scrolls, not the region.
- Bottom navigation is `position: fixed` and the scrolling region carries bottom
  padding equal to its height, so the composer and the last comment are never
  underneath it.

---

## 4. Shared component structure

The three presentations compose the same pieces. This is what prevents a fact
from existing on one surface and vanishing on another.

| Component | Row | Panel | Page |
| --- | :-: | :-: | :-: |
| `TaskIdentity` — key, type | ● | ● | ● |
| `TaskAttentionSummary` — blocked, overdue, subtask progress | ● | ● | ● |
| `TaskStatusControl` | ● (read) | ● (edit) | ● (edit) |
| `TaskPeople` — assignees | ● | ● | ● |
| `TaskDescription` | — | excerpt | full, inline edit |
| `TaskRelationships` — blocked by / blocks | secondary line | summary | full, separate sections |
| `TaskSubtasks` | progress only | `3 of 5 complete` | full list |
| `TaskComments` | — | count + latest | composer + thread |
| `TaskMetadata` | selected columns | selected rows | full definition list |
| `TaskActions` | on hover/focus | status + open | full, overflow menu |

**Terminology is shared too.** A thing called "Blocked by" on the page is
"Blocked by" in the panel and in the row's secondary line. No surface renames a
concept.

---

## 5. List row — scan and decide

The list answers *what needs my attention*. It does not attempt to display the
whole task.

### Desktop columns

Identifier + type · **Title (visually dominant)** · Status · Priority ·
Assignees · Due · Updated.

Under the title, a **secondary line** carries only what demands attention:
blocked state, overdue state, subtask progress. Nothing else.

```
┌────────┬────────────────────────────┬───────────┬──────────┬────────────┐
│ ONB-12 │ Fix mobile task layout     │ In prog.  │ Sachin   │ Tomorrow   │
│ Bug    │ Blocked by ONB-8           │ High      │          │            │
└────────┴────────────────────────────┴───────────┴──────────┴────────────┘
```

Row actions appear on hover **and on keyboard focus**, and are laid out so the
columns do not move when they appear. A row that reflows under the pointer is a
row people mis-click.

### Mobile — a stacked summary, not a crushed table

```
ONB-12 · Bug                        High
Fix mobile task layout
In progress · Sachin · Due tomorrow
Blocked by ONB-8
```

**No horizontal page scrolling, ever.** Secondary columns fold into the stacked
summary; the **title is never the column that shrinks**.

---

## 6. Side panel — quick peek

Four questions, **without scrolling**: what is this, what state is it in, who
owns it, does anything need attention.

```
┌─────────────────────────────────────────┐
│ ONB-12              Open full view   ×  │
├─────────────────────────────────────────┤
│ Fix mobile task layout                  │
│                                         │
│ [In progress ▾]  [High]                 │
│                                         │
│ Assignees     Sachin, Aditi             │
│ Due           Tomorrow                  │
│ Blocked by    ONB-8 — API contract      │
│                                         │
│ Description                             │
│ The mobile task page currently…         │
│                                         │
│ Subtasks      3 of 5 complete           │
│ Comments      4 · latest 20 min ago     │
│                                         │
│               Open full task →          │
└─────────────────────────────────────────┘
```

- **Width 480–520 px** on desktop.
- **No long forms, no full comment thread.** Description is a controlled
  excerpt. Relationships are summaries, not editors.
- **Status is editable** — frequent and compact, so it does not get deferred to
  the page.
- **"Open full task" is always obvious**, and is a real anchor so ⌘-click opens
  a tab.
- **At narrow widths there is no drawer.** Opening a task navigates to the full
  route. A drawer on a phone is a page with a worse header.

### Accessibility contract for the panel — **this supersedes the previous note**

An earlier revision of this document argued the panel should be a
`role="complementary"` region *without* a focus trap, on the grounds that
trapping would make the list behind it unreachable. That is now overruled, and
the reason is worth recording: **the panel makes its background inert.** Once
the background is inert, a trap is not a restriction — it is the honest
description of what is already true, and a focus ring that wanders into inert
content is a keyboard user lost with no way back.

So the panel is a **dialog**:

- `role="dialog"`, `aria-modal="true"`, labelled by the task title.
- Background inert.
- Focus moves into the panel on open and is **trapped**.
- `Escape` closes it, and **focus returns to the originating row or card**.

The two configurations are a matched pair. A panel that traps focus must make
its background inert; a panel that does not must not trap. They are never mixed.

---

## 7. Full page — understand and work

The canonical presentation. On desktop it uses the available width and keeps
every essential fact above the fold.

```
┌──────────────────────────────────────────────────────────────────────┐
│ ← Tasks    ONB-12 · Bug                     Watch   Share   ⋯        │
├─────────────────────────────────────────────┬────────────────────────┤
│ Fix mobile task layout                      │ Status                 │
│                                             │ [In progress ▾]        │
│ Description                          Edit   │ Assignees              │
│ The mobile task page currently…             │ Sachin, Aditi    Edit  │
│                                             │ Priority     High      │
│ Blocked by                                  │ Due          Tomorrow  │
│ ONB-8 · API contract                 Active │ Start        8 Aug     │
│                                             │ Project      Web app   │
│ Subtasks                          3/5 done  │ Reporter     Demo User │
│ ✓ Audit current layout                      │ Tags         UX, Mobile│
│ ○ Design mobile composition                 │ Milestone    Phase 1   │
│                                             │                        │
│ Comments                                    │ Created      Yesterday │
│ [Write a comment…                        ]  │                        │
│ Latest conversation…                        │                        │
└─────────────────────────────────────────────┴────────────────────────┘
```

Fixed decisions:

- The **title is the `<h1>`**. One per route.
- **Status and owner are never below the fold.**
- Description, relationships, subtasks and comments are the **working
  narrative**, in that order.
- Metadata is a **compact definition list, not a series of cards.**
- **Routine fields edit inline.**
- **Archive, delete and other uncommon actions live in the overflow menu**, not
  on the surface.
- **"Blocked by" and "Blocks" are separate sections**, because direction
  matters and merging them loses the only fact that distinguishes them.
- The **comment composer sits above the recent conversation**, immediately
  available.
- Empty sections carry compact operational copy: *"No subtasks yet. Add one."*
  — never a paragraph explaining the feature.

### The two clocks on this page

The standing line — whose court, where it reached, how it last fared — sits
directly under the title, above the description. It is the one thing here that
no other tracker can say, and it is a sentence rather than three fields.

---

## 8. Full page on mobile — a separate composition

Not reordered desktop columns.

```
┌─────────────────────────────┐
│ ← Tasks       ONB-12    ⋯   │
├─────────────────────────────┤
│ Fix mobile task layout      │
│                             │
│ [In progress ▾]  [High]     │
│ Sachin · Due tomorrow       │
│                             │
│ Description            Edit │
│ The mobile task page…       │
│                             │
│ Blocked by                  │
│ ONB-8 · API contract        │
│                             │
│ Subtasks              3/5   │
│ Comments                    │
│ [Write a comment…]          │
│ Latest conversation…        │
│                             │
│ More details            ▾   │
└─────────────────────────────┘
```

- **One page-level vertical scroll. No nested scrolling.**
- **No metadata-first layout.** Status, assignee, priority and due stay near the
  title; everything less frequent goes under **More details**.
- Every touch target ≥ 44 px.
- Bottom navigation never covers the composer or the last content.
- The **sticky header carries back, identifier and overflow only** — never
  workspace administration.

---

## 9. Accessibility contract

Binding on all three presentations:

- One `<h1>` on the full route; the panel's title labels its dialog.
- Logical heading sequence for Description, Relationships, Subtasks, Comments.
- Focus **trapping and restoration** in the panel (§6).
- Status and priority communicated with **text, not colour alone**.
- Visible focus on every editable field and every action.
- **Keyboard-operable** status changes and relationship actions.
- Errors rendered **adjacent to the field** they concern.
- **Live announcements** for saves, transitions and comment failures.
- **User input preserved after a failed mutation** — a comment that fails to
  post is still in the box.
- Verified by **real-browser** axe, keyboard, responsive and visual-regression
  tests. The jsdom suite cannot see layout, focus order or contrast, and a suite
  that claims to check accessibility while skipping those retires the question.

---

## 10. Order of work

The components are reorganised around this architecture rather than patched
individually:

1. Extract the shared components in §4 so all three presentations compose the
   same pieces.
2. Settle the scrolling model in §3 — shell first, then routes.
3. Rebuild the panel as a dialog (§6), including narrow-width navigation.
4. Rebuild the list row, desktop and mobile (§5).
5. Rebuild the full page's two compositions (§7, §8).
6. Add the real-browser test layer (§9), without which none of the above stays
   fixed.

## 11. Related

- [45](45-DEVELOPMENT-LIFECYCLE-AND-CUSTODY.md) — the two clocks and the chain
  of custody, which the standing line states.
- [44](44-PRODUCT-RESEARCH-AND-SURFACE-BRIEFS.md) — the jobs and their time
  budgets.
- [42](42-FRONTEND-ARCHITECTURE.md) — the bundle budget behind "no editor in the
  panel".
