# 45 — The development lifecycle, custody and environments

How work actually moves through a team that has QA, several platform teams and a
deployment pipeline — and what that forces into the model.

Read [44](44-PRODUCT-RESEARCH-AND-SURFACE-BRIEFS.md) first: it establishes who
opens this product and at what moment. This document is the *process* those
people are inside.

## The problem with one clock

A task tracker built on a single status column has to choose between two facts:

- **what state the work is in** — open, being worked, resolved, verified;
- **where the work has reached** — dev, qa, staging, production.

Collapsing them produces columns like `In QA`, `On Staging`, `In Prod`, and then
cannot express the ordinary case: *resolved, sitting on qa, verified there, not
yet promoted to staging*. It also cannot answer the question a release
conversation is entirely made of — **what is on staging right now** — because
"on staging" was spent describing progress instead.

So the model carries **two clocks**, and they advance independently:

| Clock | Advanced by | Values |
| --- | --- | --- |
| **Status** | the person holding the task | the workflow ([23](23-WORKFLOW-AND-STATE-MACHINE.md)) |
| **Environment** | a deploy or a release | `project_environment`, in `position` order |

`task.environment_id` is *where it is now*. The promotion log is *how it got
there*. Neither is a status.

## The chain of custody

The lifecycle is a sequence of hand-offs. Each one is an event with an owner
before and an owner after — which is the thing the product must make visible,
because "whose turn is it" is what every role opens the tool to find out.

| # | Stage | Who acts | What changes |
| --- | --- | --- | --- |
| 1 | **Intake** | QA, management, support | Task exists: type, severity, the environment it was *seen* on, suspected team |
| 2 | **Triage** | Lead, QA lead | Owning **team** set or corrected. This repeats — a bug filed against Android turns out to be Backend's |
| 3 | **Assignment** | Team lead, or self-serve | An assignee inside the owning team |
| 4 | **Work** | Developer | Status only. Nothing else moves |
| 5 | **Resolve and push** | Developer | Status → resolved, environment → the one it was deployed to, **with proof** |
| 6 | **Verify** | QA, *on that environment* | Pass, or fail with evidence. A fail returns custody and is counted |
| 7 | **Promote** | A release | Environment advances: qa → staging → production. Many tasks at once |
| 8 | **Close** | QA or lead | Verified on production |

Stages 2, 5, 6 and 7 are the ones the current product cannot express at all.

## What this adds to the model

Five things. Every one of them exists because a stage above cannot be recorded
without it — none is speculative.

### 1. A task has an owning team

`task.team_id`, nullable, referencing `team`. Nullable because stage 1 happens
before stage 2: an untriaged task belongs to no team yet, and **that is the
triage queue** — the most useful list a lead has.

Multi-team projects already exist (`project_team`, migration 0029), so the team
must be one of the project's teams; anything else is a task owned by people who
cannot see it.

### 2. Transfers are logged, not just applied

`task_team_transfer`: which task, from which team, to which, by whom, when, and
why. Applied to `task.team_id` in the same transaction.

The log is not bookkeeping. **The bounce count is the number that exposes a
broken process** — a bug that has crossed between Android and Backend three times
is a specification problem, not an engineering one, and no product surfaces it
because no product records it.

**On transfer the assignee is cleared.** The task lands unassigned in the
receiving team's queue, and their process picks it up. The alternative — keeping
the previous developer attached — leaves the receiving team with nothing in any
queue to notice, which is the failure teams complain about most.

### 3. Environment promotions are logged

`task_environment_promotion`: which task, which environment, by whom, when, and
the release it went with if any. `task.environment_id` stays as the current
value, so every existing filter and the `EnvironmentIn` constraint keep working
unchanged.

This is what makes these answerable, and none of them is today:

- what is on staging right now, and what is not yet;
- when did WR-125 reach production;
- how long does a fix take to get from qa to production (the flow metric that
  actually predicts a release date).

### 4. A release is a batch promotion

`release`: a named set inside a project — "2026.08.1" — that promotes many tasks
to one environment at once, writing one promotion row per task.

Both halves are needed, and that is a deliberate choice rather than an
either/or: **resolve sets the first environment** (the developer says where they
pushed it, per-ticket), and **a release promotes a batch** (staging, then
production, for everything that went out together). Teams do both, at different
moments, and a model with only one of them forces the other to be faked.

### 5. Verification is an outcome, not a status change

`task_verification`: task, environment, verdict (`PASS` / `FAIL`), who, when, and
the evidence.

A fail is not "moved back to In Progress" — that is what happens *as a result*.
The fact worth keeping is that it was tested on qa and failed, with the evidence,
because "failed verification twice on the same environment" is a sentence a
status column can never produce.

## Whose turn is it

The derived field the whole product organises around. It is **computed, never
stored** — a stored copy is a cache of three columns that will disagree with them.

| Condition | In the court of |
| --- | --- |
| No owning team | **Triage** — a lead |
| Team set, no assignee | **The team** — its queue |
| Assigned, status not resolved | **That developer** |
| Resolved, not yet verified on its environment | **QA** |
| Verified on the last environment | **Nobody** — it is done |

Every role's home screen is a filter over this one field, which is why it belongs
in the domain and not in a view:

- a **developer** sees *in my court*, then *my team's queue*;
- **QA** sees *waiting on me to verify*, grouped by environment, because they
  test an environment at a time and switching costs them;
- a **lead** sees *in nobody's court*, *bounced more than once*, and *resolved
  but not promoted* — the three shapes of stuck.

## Permissions

No new mechanism. `docs/04`'s closed constraint set already carries a
parameterised member — `EnvironmentIn(Vec<EnvironmentId>)` — and per-type
creation follows it exactly:

```
Constraint::TaskTypeIn(Vec<TaskType>)
```

A developer holds `task.create` constrained to `TaskTypeIn([BUG, INCIDENT])`:
they may raise a bug against their own work and may not invent a feature. QA and
management hold it unconstrained. This adds one variant and one fact
(`ResourceFacts.task_type`) rather than a `task.create.bug` key per type, which
would multiply the registry by the type list and break the moment a workspace
adds a type.

**Transfer and promotion are `task.update`.** They change how a task behaves and
who is answerable for it, which is what that permission means — the same reading
`docs/23` applied when dependencies chose it over inventing
`task.dependency.add`. Verification is `task.transition`, because a verdict
always ends in a transition and a QA who could verify but not move the task would
be stuck holding a result.

### Role templates, as the lifecycle implies them

| Role | Holds | Notably does not |
| --- | --- | --- |
| **Developer** | `task.read`, `task.update`, `task.transition`, `task.comment`, `task.create` **constrained to BUG, INCIDENT** | Create features; verify |
| **QA** | `task.create` unconstrained, `task.transition`, `task.comment`, `task.update` | Administer |
| **Lead** | QA's, plus `task.assign`, `task.delete`, `project.update` | Workspace administration |
| **Manager** | Read across projects, `audit.read`, reports | Change work |

## What the surfaces become

Derived from [44](44-PRODUCT-RESEARCH-AND-SURFACE-BRIEFS.md)'s briefs, now that
the process is written down.

- **Home** — "in my court", by role. The single highest-frequency job.
- **Environment board** — environments as columns, tasks as cards. The answer to
  "what is on staging", and it did not exist.
- **Team view** — one team's queue, in-flight and bounced work.
- **Item** — adds custody: owning team, transfer history, environment history,
  verification results. Resolve requires the environment and the proof.
- **Release** — what went out together, and where it is now.

## Open questions

1. **Should a task be transferable to a team not on its project?** Today it must
   not be, because the team could not see it. The alternative is that transfer
   adds the team to the project, which quietly widens visibility. *Left refusing;
   revisit if teams hit it.*
2. **Does a failed verification return the task to the developer who resolved it,
   or to the team queue?** Returning to the person is faster; returning to the
   queue survives holidays. *Assumed: the person, since they have the context —
   flagged for the first team that disagrees.*
3. **Are releases per project or per workspace?** Modelled per project, because
   environments are. A workspace-wide release train needs the wider shape.
