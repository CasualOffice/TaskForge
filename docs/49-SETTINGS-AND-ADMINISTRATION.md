# 49 — Settings and Administration

Settings was assembled the same way the task surface was: one entry per screen
that existed, in the order they were written, with the shape each screen
happened to need. The result is navigable by someone who already knows what is
in it, which is the definition of an interface that was never designed.

The complaint, in full: *confusing, non-navigable for new users, no standard
followed, no UX planned.* All four are fair, and each has a specific cause.

---

## 1. What is actually wrong

| Symptom | Cause |
| --- | --- |
| A new user cannot tell which settings are theirs and which are everyone's | One flat list mixes **my account** with **workspace administration**. They are different scopes with different permissions and different consequences. |
| Every settings page's `<h1>` says "Settings" | The heading belongs to the *navigation*, so no page ever names itself. The page identity is missing, not just small. |
| The navigation is a wall of text | Seven entries × two lines of description each. Descriptions belong on the page they describe, not in the list you scan to find it. |
| Roles is unreadable | It renders **raw permission keys** — `task.dependency.override`, `workspace.manage` — as prose. Twenty-nine of them, in one paragraph. |
| Primary actions are hard to find | "New role" sits *below* the list it creates into. |
| Nothing is bounded | Each page is one long scroll of unrelated groups with no separation, the same defect `docs/47` fixed on the task page. |
| Three navigations compete | App sidebar, settings list, and content, side by side, all at the same visual weight. |

---

## 2. Competitive reading

> Written from working knowledge of these products, not fetched documentation.

**GitHub.** Left navigation grouped under headings, with the *scope* stated:
personal settings are visibly separate from organisation settings. Each page
carries its own `<h1>` and a one-line description. Sections are bounded panels
with a single action each.

**Linear.** Two groups only — *Workspace* and *My account* — and nothing else
competing. Page header names the section; content is bounded rows.

**Stripe, Slack.** Grouped navigation, page `<h1>`, section cards, and a save
affordance that is adjacent to what it saves rather than floating at the bottom
of the scroll.

**Notion.** Settings in a modal, which they can do because their field set is
small and their sections are shallow. Ours are neither, which is why settings
here is a route tree — and that decision, already recorded in
`SettingsLayout`, is right and stays.

**The standard all of them follow:** *group by scope, name the page, bound the
sections, put the action where the thing is.* None of the four was being done.

---

## 3. Information architecture

Two groups, because there are exactly two scopes and a person needs to know
which one they are changing.

```
Account
  Your profile          name, time zone, password, sessions

Workspace
  General               name and identifier
  Members               people and invitations
  Teams                 groups a grant can name
  Roles                 what each role carries
  Workflow              statuses and the moves between them
  Tags                  the shared vocabulary
```

- **Account** changes affect only the signed-in person.
- **Workspace** changes affect everyone, and most require a permission.

The group headings are the whole mechanism. "Am I about to change something for
everybody?" is answerable at a glance, and it was not.

**Entries are still not filtered by permission.** A section you cannot
administer shows the sentence that says so. Hiding it would leave someone told
"your admin can change that under Roles" staring at a navigation with no Roles
in it, unable to tell whether the feature is missing or they are. That reasoning
already exists in the code and it survives this redesign unchanged.

---

## 4. The page template

Every settings page is the same three things:

```
┌──────────────────────────────────────────────────────────┐
│ Roles                                        [ New role ]│  h1 + primary action
│ A role is a named set of permissions.                    │  one line, plain
├──────────────────────────────────────────────────────────┤
│ ┌ Administrator                              27 · Edit ┐ │  bounded section
│ │ Can administer the workspace, manage projects…       │ │  English, not keys
│ └──────────────────────────────────────────────────────┘ │
│ ┌ Guest                                        2 · Edit ┐ │
│ └──────────────────────────────────────────────────────┘ │
└──────────────────────────────────────────────────────────┘
```

Rules:

1. **The page carries the `<h1>`**, and it names the section. The navigation's
   own heading drops to a label — a nav is not the page.
2. **One line of description**, on the page, under the heading. It is removed
   from the navigation entry, which becomes a single scannable line.
3. **The primary action sits with the heading, top-right** — never below the
   list it adds to.
4. **Groups are bounded sections**, separated by space first and a border only
   where space is not enough.
5. **The save affordance is adjacent to what it saves.**
6. **Errors render next to the field**, not at the top of the page.

---

## 5. Permissions in words

The single worst thing on the surface. `task.dependency.override` is an
identifier; it is not language. A workspace owner deciding who may do what is
reading twenty-nine of them in a paragraph and can act on none of it.

The rule: **a permission is shown by its name, and the key is available but not
the default.**

| Key | Shown as |
| --- | --- |
| `task.create` | Create tasks |
| `task.dependency.override` | Force a transition past a blocker |
| `workspace.manage` | Change workspace settings |
| … | … |

A role lists **what it can do, grouped by the thing it acts on** — Tasks,
Projects, Workspace, Administration — and shows a count with the first few
names, not all of them. The full list belongs in the editor, where someone is
deliberately reading it.

The permission registry is closed (`docs/04`), so the mapping is exhaustive and
a missing name is a build-time gap rather than a blank on screen.

**No engineering artefacts anywhere on these surfaces**: no permission keys as
prose, no decision IDs, no endpoint names, no schema limits, no tracker
references. They belong in `docs/` and in development diagnostics.

---

## 6. Accessibility

- **One `<h1>` per route, and it names the section** — not the navigation.
- The settings navigation is a `<nav>` with an accessible name; its own heading
  is a label, not an `<h1>`.
- The active entry is marked with `aria-current="page"` and **matched exactly**,
  so one entry is current and never two.
- Any scrollable region is keyboard-focusable, or it is not scrollable.
- Every control meets the 44 px touch tier on coarse pointers.
- Field errors are adjacent to their field and announced.

---

## 7. Order of work

1. Group the navigation by scope; drop the per-entry descriptions; exact-match
   the active state.
2. Give every page its own header — `<h1>`, one-line description, primary
   action top-right.
3. Translate permissions into language, everywhere they are shown.
4. Bound the sections on each page.
5. First-run and empty states, including "New workspace" actually creating one.

## 8. Related

- [47](47-TASK-SURFACE-TEMPLATE.md) — the same argument applied to the task
  surface; the section template here follows its bounded-region rule.
- [04](04-AUTHORIZATION-MODEL.md) — the closed permission registry the language
  table in §5 is built from.
