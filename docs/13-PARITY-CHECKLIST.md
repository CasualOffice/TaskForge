# 13 — Category Parity Checklist

**Status:** Working checklist, updated as rows move.
**Purpose:** One place that answers "can a team actually run their work in this?"

## What this is, and the constraint it operates under

[12](12-COMPETITIVE-ANALYSIS.md) names OrangeScrum "the clean-room reference
point": we study its **feature surface** to understand what self-hosting teams
expect, and we copy **no source, schema, template, or asset**
([01](01-ORD.md) NFR-9, [09](09-REPOSITORY-AND-CONTRIBUTION.md)). That document
reduces the expectation to a checklist — *projects, tasks, time, milestones,
roles* — and is explicit that it is "a checklist of what to have an answer for,
not a design to imitate."

This file is that checklist, extended to the whole surface, scored against what
is actually reachable by a user in a browser.

**The scoring rule is deliberately harsh.** A capability counts as `Usable` only
when a person can do it end to end from the UI. An endpoint with no interface is
`API only` — which is worth recording honestly, because it is a very different
amount of remaining work from nothing at all, and most of this product is
currently in that state.

| Score | Meaning |
| --- | --- |
| `Usable` | A user can do it from the browser, end to end |
| `API only` | Server built and tested; nothing renders it |
| `Partial` | Reachable but incomplete or one-directional |
| `Model only` | Schema exists; no endpoint |
| `None` | Neither |

## Getting started

| Capability | API | UI | Score | Owner |
| --- | --- | --- | --- | --- |
| **First-run onboarding** — create a workspace, invite a team, create a project | Endpoints exist | ❌ | `None` | onboarding |
| Empty-state guidance on every surface | n/a | ❌ | `None` | UX |

A new user today signs in and sees an empty list with no route to a first
workspace. Every endpoint the flow needs already works; nothing sequences them.

## Core work management

| Capability | API | UI | Score | Owner |
| --- | --- | --- | --- | --- |
| Workspaces — create, switch, rename | ✅ | ❌ | `API only` | settings surface |
| Projects — create, list, visibility | ✅ | Partial | `Partial` | — |
| Tasks — create, edit, delete | ✅ | Partial | `Partial` | UX |
| Task key (`WR-125`) | ✅ | ✅ | `Usable` | — |
| Statuses and transitions | ✅ | ✅ | `Usable` | — |
| Board with drag-and-drop | ✅ | ✅ | `Usable` | — |
| List view, virtualised, keyset paged | ✅ | ✅ | `Usable` | — |
| My Work | ✅ | ✅ | `Usable` | — |
| **Subtasks** | Partial — `parent_id` on create, nothing reads a tree | ❌ | `Partial` | **unassigned** |
| **Dependencies / blockers** | ❌ | ❌ | `None` | relations |
| **Milestones** | ❌ — table exists, no endpoint | ❌ | `Model only` | **unassigned** |
| **Time tracking** | ❌ | ❌ | `None` | **unassigned — and undesigned** |
| Tags | ✅ | ❌ | `API only` | **unassigned** |
| Assignees — write | ✅ | Partial | `Partial` | — |
| Assignees — read | ❌ | ❌ | `None` | relations |
| Comments | ✅ | ✅ | `Usable` | — |
| Attachments | Partial | ❌ | `Partial` | C-010 |
| Activity / history | ❌ | ❌ | `None` | relations |
| Search, full-text | ✅ | ✅ | `Usable` | — |
| Filters | ✅ | ✅ | `Usable` | filters |
| Saved views | Built-ins only; no `/saved-views` endpoint | Partial | `Partial` | filters |
| Group and sort | Sort ✅, group ❌ | Partial | `Partial` | filters |
| Live updates (SSE) | ✅ | ✅ | `Usable` | — |

## People and administration

| Capability | API | UI | Score | Owner |
| --- | --- | --- | --- | --- |
| Sign in / out, sessions | ✅ | ✅ | `Usable` | — |
| Password reset by email | ✅ | Partial | `Partial` | — |
| MFA | ✅ | ❌ | `API only` | — |
| **Profile — name, email, timezone** | ❌ | ❌ | `None` | profile |
| **Password change** | ❌ | ❌ | `None` | profile |
| **Session list and revoke** | Partial | ❌ | `Partial` | profile |
| Members — list, add, remove | ✅ | ❌ | `API only` | settings surface |
| Invitations — send, revoke | ✅ | ❌ | `API only` | settings surface |
| Teams | ✅ | ❌ | `API only` | settings surface |
| **Roles — author, assign** | ❌ | ❌ | `None` | roles API |
| Permission explain | ✅ | Partial | `Partial` | — |
| Workflow authoring — add/rename/reorder a status | ❌ | ❌ | `None` | workflow config |
| Workflow authoring — transitions, required fields | ❌ | ❌ | `None` | workflow config |
| Delete a status with a migration target | ❌ — fully specified in [23](23-WORKFLOW-AND-STATE-MACHINE.md) | ❌ | `None` | workflow config |
| Environments — add, assign to a task | ❌ — `project_environment` table exists | ❌ | `Model only` | workflow config |
| Admin rights — who may administer what | Resolver ✅, no authoring | ❌ | `Partial` | roles API |

## Reporting and output

| Capability | API | UI | Score | Owner |
| --- | --- | --- | --- | --- |
| Notifications — in-app | ✅ | ❌ | `API only` | **unassigned** |
| Notifications — email | ✅ | n/a | `Usable` | — |
| Notification preferences | ❌ — D-059 open | ❌ | `None` | blocked on D-059 |
| Personal dashboard (My Week) | ❌ | ❌ | `None` | reports |
| Team dashboard (Team Workload) | ❌ | ❌ | `None` | reports |
| Project dashboard (Project Health) | ❌ | ❌ | `None` | reports |
| Workspace dashboard | ❌ — no built-in defined at this scope | ❌ | `None` | reports |
| Reports and measures | ❌ | ❌ | `None` | reports |
| Export CSV / JSONL | ❌ | ❌ | `None` | export |

## The two genuine design gaps

Everything above is either built, assigned, or a known deferral. Two are
neither, and both are on the parity checklist [12](12-COMPETITIVE-ANALYSIS.md)
names:

### Time tracking — undesigned

Not in [01](01-ORD.md) FR-1 to FR-13, and **not in its non-goals either**. It is
simply absent from the design record while sitting in the middle of the category
baseline. Self-hosting teams choose products in this category partly to answer
"where did the week go", and there is no answer here at all.

It is a real design decision, not an oversight to code around, because the
shape matters: a duration on a task, or timed entries with a start and stop; who
may see whose time; whether an estimate is a separate field; whether it feeds
`cycle_time` in [38](38-REPORTING-EXPORT-AND-DASHBOARDS.md), which currently
derives everything from state intervals rather than logged effort. Answering it
in code first would settle all of that by accident.

**Recorded as D-062, open.** Needs a decision before implementation.

### Milestones — modelled and unreachable

`migrations/0004` creates the `milestone` table and `migrations/0005` gives
`task.milestone_id` its own partial index, so the schema is committed to it and
[01](01-ORD.md) FR-3 names it. Nothing reads or writes either. This one is not a
design question — the model already decided — it is unbuilt.

## What parity does not mean

[12](12-COMPETITIVE-ANALYSIS.md) §"Where TaskForge deliberately differs" and
[01](01-ORD.md) §Non-goals both stand. Parity is the *work-management* surface,
not everything a competitor bundles. Out of scope by decision, not by omission:
CRM, invoicing, payroll, chat, video, whiteboards, document editing, and
Gantt/portfolio/resource management — the last of which is deliberately a plugin
surface rather than a core feature.
