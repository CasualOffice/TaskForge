# TaskForge Frontend Requirements

**Status:** Requirements only. Layout and visual design are supplied separately
(`DESIGN-FOUNDATION.md`, `LAYOUT-AND-INTERACTION-GUIDELINES.md`,
`VISUAL-IDENTITY.md`).
**Scope:** What the client must let a person do. Not how it looks.

## End goal

A team can run their work in TaskForge without ever touching the API directly.

Concretely: sign up, set up a workspace, invite colleagues, organise them into
teams, create projects, plan and execute work on them, see where the work is,
and administer the whole thing — from a browser, without a developer.

Nothing that the server can do may be reachable only by an API call.

## Non-negotiable properties

These are properties of the whole client, not of any one screen.

1. **Authority is the server's.** The client renders affordances from
   `GET /api/v1/permissions/effective` and never decides permission itself. A
   hidden button is not a control; an absent one is not security.
2. **Every refusal is legible.** A rejected action shows what was refused, why,
   the `docs/20` code, and the request id. Never a generic failure.
3. **A URL is the state.** Filters, sort, grouping, the open task and the
   selected project all live in the URL, so any view can be shared,
   bookmarked, and returned to with the back button.
4. **Symbols stay symbolic.** `@me` and `+7d` are sent as written and resolved
   by the server, so a shared link is correct for whoever opens it.
5. **Optimistic where safe, never silent.** An optimistic change that the
   server refuses is rolled back visibly with the reason.
6. **Live, not polled.** Server-sent events patch the affected surface; the
   client does not reload a view to notice a change.
7. **Keyboard-complete.** Every core flow is operable without a pointer,
   including anything that is drag-and-drop with a mouse.
8. **Accessible.** WCAG 2.2 AA is an acceptance criterion per component, not a
   release task.
9. **Bounded.** Initial authenticated shell ≤ 200 KiB gzip (ADR-024).
   Administration, reporting and settings are lazy.

## Required capabilities

Each row is done when a person can complete it end to end in a browser.

### Getting started

| # | Requirement |
| --- | --- |
| G1 | Sign in, sign out, and recover a forgotten password by email |
| G2 | Enrol and use a second factor, and use a recovery code |
| G3 | A new user with no workspace can create one and land in a usable product |
| G4 | A user invited to an existing workspace joins it without seeing setup they do not need |
| G5 | Every step of setup is skippable and resumable later |

### Work

| # | Requirement |
| --- | --- |
| W1 | Create a task from anywhere, choosing its project |
| W2 | Read a task in full — what it is, who owns it, its state, dates, relationships and conversation — without hunting for any of it |
| W3 | Edit a task's fields, deliberately, with the change confirmed or refused visibly |
| W4 | Move a task through its workflow, and be told before the attempt when a move is not permitted |
| W5 | Assign and unassign people, and see who is assigned |
| W6 | Comment, reply, edit and delete own comments |
| W7 | Attach a file, download it, and see when one is not yet available |
| W8 | Create subtasks and see a parent's progress, without any implication that children drive the parent's status |
| W9 | Declare that one task blocks another, see both directions, and be refused a cycle with the loop named |
| W10 | Tag a task, and find tasks by tag |
| W11 | Put a task on a milestone, and see a milestone's progress |
| W12 | See a task's full history |

### Finding

| # | Requirement |
| --- | --- |
| F1 | Full-text search across tasks |
| F2 | Filter on every field the server's grammar supports, with active filters legible in plain language |
| F3 | Group and sort a list |
| F4 | Save a filter as a named view, share it, and use the built-in views |
| F5 | See the same work as a list or a board, and move cards on the board |
| F6 | See only my own work, without constructing a filter to do it |
| F7 | Select several tasks and act on them at once |
| F8 | Export the current view as a file |

### People and structure

| # | Requirement |
| --- | --- |
| P1 | See and edit own profile: name, email, timezone |
| P2 | Change own password; see and revoke own sessions |
| P3 | Switch between workspaces, and create another |
| P4 | Rename a workspace and see who is in it |
| P5 | Invite someone by email with a role, and revoke a pending invitation |
| P6 | Remove a member, and be told when that is refused and why |
| P7 | Create teams and manage their membership |
| P8 | Put several teams on one project, and see which projects a team works on |
| P9 | Create, configure and archive projects |
| P10 | See a project: its work, people, milestones, environments and workflow |
| P11 | See a team: its people, its projects and its work |

### Administration

| # | Requirement |
| --- | --- |
| A1 | Author roles and see what permissions each grants |
| A2 | Grant and revoke roles at workspace, team, project and environment scope |
| A3 | Ask why a person can or cannot do something, and get the contributing grants back |
| A4 | Add, rename, reorder and remove workflow statuses, and define transitions |
| A5 | Remove a status by choosing where its tasks go |
| A6 | Manage a project's environments |
| A7 | Reach every administrative surface from one place, without a permanent slot for each |

### Awareness

| # | Requirement |
| --- | --- |
| N1 | See in-app notifications with an unread count, and read them |
| N2 | Control what generates a notification and how it arrives |
| N3 | See a personal dashboard: my open work, what is overdue, what is coming |
| N4 | See a team dashboard: load and gaps across its people |
| N5 | See a project dashboard: throughput, cycle time, blocked work, ageing |
| N6 | See a workspace dashboard: where work sits across projects and teams, and where it is stuck |

## States that must exist everywhere

Not decoration — these are the states a user actually meets.

| State | Requirement |
| --- | --- |
| Empty | Says which empty it is and what to do next |
| Loading | Holds geometry; no full-screen loader after the shell has started |
| Error | Names what failed, with the code and request id |
| Refused | Explains the missing authority, not just "forbidden" |
| Not yet built | Says so once, plainly, without internal identifiers |
| Restricted | Content the viewer may not see is named as restricted, never as missing |
| Offline | Says what is stale and what will retry |

## Out of scope

Deliberately not the client's job:

- Deciding permission. It renders what the server says.
- Storing anything the server owns. Cache, never a second source of truth.
- Report definitions beyond the closed measure set.
- Anything in `docs/01` §Non-goals: CRM, invoicing, payroll, chat, video,
  whiteboards, document editing, Gantt and resource management.

## Acceptance

A requirement is met when:

1. A person completes it in a browser against a real server.
2. It works from the keyboard alone.
3. Its empty, loading, error and refused states all exist.
4. It renders no control the actor lacks authority for.
5. The a11y gate passes over it.
6. The bundle stays within budget.

Anything that cannot yet be met is stated on screen where the user would look
for it — once, plainly, in the product's own voice.
