# 46 — The Task Surface: Template, Research and Fundamentals

The task is the only object in this product that every role opens every day. It
has two surfaces — a **peek** and a **page** — and until now neither was
designed. Both were assembled from the fields the API returns, in the order it
returns them, which is why the result reads as a form rather than as a document.

This closes that. It is a template, not a suggestion: the regions, their order,
their spacing and their responsive behaviour are fixed here, and both surfaces
are built from it.

> **Source note.** The competitive section is written from working knowledge of
> these products, not from fetched documentation. Where a claim is a measured
> fact it is marked `[Measured]`; where it is an observed pattern it is marked
> `[Pattern]`. Nothing here is a quotation of anyone's private design system.

---

## 1. What the two surfaces are for

The failure they replace is that both tried to do the same job at different
sizes. A panel that is a small version of the page is a page you have to scroll
in a smaller box.

| | Peek (side panel) | Page (full route) |
| --- | --- | --- |
| **The question** | "Is this the one I mean, and where does it stand?" | "What is this, what happened, what do I do about it?" |
| **Budget** | About five seconds, from a list | Minutes, deliberately |
| **Enters from** | Clicking a row; `Escape` returns | ⌘-click, a pasted link, "Open full view" |
| **Carries** | Identity, standing, description excerpt, assignment, environment | Everything, including every act and the whole history |
| **Never carries** | Multi-step forms, history, comments composer | — |

The rule that follows: **the peek answers, the page transacts.** If a control on
the peek would need a second decision after pressing it, it belongs on the page.

---

## 2. Competitive research

### Linear — issue peek `[Pattern]`

Right-hand overlay from a list. Its whole content above the fold is: identifier,
title, a horizontal row of *property pills* (status, assignee, priority,
labels), then description. Properties are pills rather than label/value rows,
which is what lets eight of them fit in one band instead of eight lines.

**What we take:** the property band. **What we leave:** the density — Linear's
pills assume a user who has learned the icons.

### Jira — issue view `[Pattern]`

Two columns: the narrative left, a "Details" accordion right. The details panel
groups fields under collapsible headings and puts *People* and *Dates* in
separate groups. Jira's own redesign moved status out of the details panel and
up beside the title, because it is the one field people look for first.

**What we take:** grouping the rail by question, and status out of the rail.
**What we leave:** the accordions — a field you have to open is a field you will
not read.

### GitHub — issue page `[Pattern]`

Single narrative column with a right rail. The rail is quiet: labels, assignees,
projects, milestone. The body is a timeline, and **the composer is at the end of
the timeline**, not pinned. GitHub's hierarchy is carried almost entirely by
whitespace and one heading weight, with very few boxes.

**What we take:** the composer at the end of the story; restraint with borders.

### Asana — task pane `[Pattern]`

The pane *is* the page — same component, docked. Fields are stacked label/value
at the top, description below, comments at the bottom. Asana can do this because
its field set is small; ours is not, which is the reason our current pane reads
as a property dump.

**What we take:** the confirmation that one component for both sizes only works
when the field set is small. Ours needs two.

### Height, Shortcut `[Pattern]`

Both put a single-sentence status line under the title — "In review · assigned
to X" — before any field list. It is the pattern closest to what `docs/45`
already needs us to say.

### What none of them have

**The two clocks.** No mainstream tracker separates *what state the work is in*
from *where it has reached*, and none shows a chain of custody between teams.
That is our differentiator (`docs/45`), so it gets the most valuable line on
both surfaces rather than being hidden in a panel.

---

## 3. Design fundamentals this template applies

1. **One question per region.** A region that answers two questions gets split.
2. **Hierarchy is size, weight and space — in that order.** Colour is a fourth
   resort and never the only signal (WCAG 2.2 §1.4.1).
3. **A heading must outrank its own contents.** Our section headings were 12 px
   uppercase muted above 14 px form labels, which inverts the hierarchy and is
   the direct cause of "the layout is messy".
4. **The 8-point grid.** Every gap is a multiple of 4, and vertical rhythm
   between regions is larger than rhythm within them — 24 between, 8 within. A
   reader parses grouping from spacing before they read a word (proximity, the
   first Gestalt principle that applies to a form).
5. **Borders are the last grouping tool, not the first.** Space groups; a border
   is for when space is unavailable. Boxes inside boxes are a smell.
6. **Progressive disclosure.** Rare, consequential acts are closed by default
   and open in place. An always-open form spends the reader's attention on a
   decision they are not making.
7. **Measure.** Prose is capped near 70 characters; a description that runs the
   width of a 1440 px window is a description nobody finishes.
8. **Touch targets are 44×44 CSS px** on coarse pointers (WCAG 2.2 §2.5.8 sets
   24 as the minimum; 44 is the tier this repository already commits to).
9. **The mobile order is the reading order.** Reflow (WCAG 2.2 §1.4.10) is not
   satisfied by putting the same regions in a column — it is satisfied when the
   column is ordered by what the reader needs first. Metadata before the title
   fails this even though it "fits".

---

## 4. The template

### 4.1 Peek — the overview

```
┌────────────────────────────────────────────┐
│ ONB-2   Task                        [✕]    │  identity bar, 44px
├────────────────────────────────────────────┤
│ Write the client                           │  h2, 18/24, 2-line clamp
│                                            │
│ In Backend's court · reached qa · failed   │  standing, one line
│                                            │
│ ● Backlog   Nobody   No priority   qa      │  property band, wrapping
│                                            │
│ The client crashes when the device is      │  description excerpt,
│ rotated during the login handshake…        │  4-line clamp, no editor
│                                            │
│ 3 comments · 1 subtask · 0 blockers        │  what is inside, as counts
│                                            │
│ [ Open full view ]                         │  one primary action
└────────────────────────────────────────────┘
```

**Rules.** No editing. No history. No composer. The counts are the honest way to
say "there is more here" without rendering it. `Open full view` is a real
anchor, so ⌘-click works.

**Accessibility.** The peek is a **complementary region, not a dialog**: it does
not trap focus, because it does not obscure the list it came from and a trap
would make the list unreachable by keyboard. It therefore carries
`role="complementary"` with an `aria-label` naming the task, `Escape` closes it,
and focus returns to the row that opened it. If it ever becomes an overlay that
covers the list, it becomes `role="dialog" aria-modal="true"` and gains a trap —
the two are a pair and must not be mixed.

### 4.2 Page — the document

```
 ← ONB   ONB-2   Task                                     [rail]
─────────────────────────────────────────────────────  ┌──────────┐
 Write the client                          Rename      │ Status   │
                                                       │ Assignees│
 In Backend's court · reached qa · failed on qa        ├──────────┤
 [Hand over…] [Record promotion…] [Verify…]            │ Priority │
                                                       │ Type     │
 ┌─ Description ─────────────────────────────┐         ├──────────┤
 │ …                                          │        │ Project  │
 └────────────────────────────────────────────┘        │ Due      │
 ┌─ Subtasks ────────────────────────────────┐         │ Start    │
 ┌─ Blockers ────────────────────────────────┐         ├──────────┤
 ┌─ Chain of custody ────────────────────────┐         │ raised by│
 ┌─ Comments ────────────────────────────────┐         └──────────┘
```

**Region order, and why.**

1. **Identity** — back, key, type. Persistent, quiet.
2. **Title** — the largest thing on the page.
3. **Standing** — whose court, where it reached, how it last fared. The product's
   own question, in a sentence, before any control.
4. **Acts** — closed. Rare and consequential; each opens in place.
5. **Description** — what it is.
6. **Subtasks**, **Blockers** — what it contains and what holds it up.
7. **Chain of custody**, **Comments** — what happened, ending with the composer.

**The rail** answers three questions in three groups, in this order: *whose turn*
(status, assignees), *what kind* (priority, type), *where and when* (project,
due, start). Provenance — raised by, created, updated — is a footnote in
smaller, muted type, not three rows the size of the status.

### 4.3 Responsive

| Width | Layout |
| --- | --- |
| ≥ 1024 px | Two columns, rail 320 px fixed, narrative `minmax(0, 1fr)` |
| 640–1023 px | One column. **Title and standing first**, then a horizontal property band in place of the rail, then the sections |
| < 640 px | As above; acts become full-width buttons; every control ≥ 44 px |

The rail **never** comes first. The current `order: -1` at narrow widths is the
defect this row exists to forbid: it puts eight fields between the reader and
the title of the thing they opened.

The page scrolls as **one** region at every width. There is exactly one
scrolling container on the route.

### 4.4 Spacing and type

| Token | Value | Used for |
| --- | --- | --- |
| Region gap | 24 px | between sections |
| Inner gap | 8 px | between a heading and its content |
| Section padding | 12 / 16 px | inside a bounded section |
| Title | 28/34, 600 | the task title |
| Section heading | 15/20, 600, sentence case | section names |
| Body | 14/21 | description, comments |
| Meta | 13/18, muted | rail labels, timestamps |

Section headings are **sentence case, not uppercase**: uppercase costs legibility
and, at 12 px muted, lost to the form labels beneath it.

---

## 5. Acceptance

- No region on either surface renders a field the template does not place.
- At 390 px the title is the first thing under the identity bar on the page, and
  the peek does not exceed the viewport.
- Every interactive element on a coarse pointer is ≥ 44 px.
- The peek is reachable, dismissable and returns focus, without trapping it.
- One scrolling container per route.

## 6. Related

- [45](45-DEVELOPMENT-LIFECYCLE-AND-CUSTODY.md) — the two clocks and the chain of
  custody, which is what the standing line says.
- [44](44-PRODUCT-RESEARCH-AND-SURFACE-BRIEFS.md) — the jobs and their time
  budgets, which is where the peek's five seconds comes from.
- [42](42-FRONTEND-ARCHITECTURE.md) — the bundle budget the "no chart library"
  and "no editor in the peek" rules protect.
