# 44 — Product research and surface briefs

Who uses this, what they are trying to do when they open it, and therefore why
each screen exists and what belongs on it.

## Why this document exists

It was missing, and its absence is visible in the product.

Three findings, each checkable:

1. **[01](01-ORD.md) §Users lists buyer segments, not people.** "Delivery teams
   (5–500 people)", "self-hosting organizations", "integrators". Those are who
   *pays*. None of them is a person with a job, a moment, or a decision.
2. **[42](42-FRONTEND-ARCHITECTURE.md) is entirely technical.** Fourteen
   sections: stack, bundle budget, rendering strategy, optimistic mutation, live
   updates, accessibility, testing. Not one is about what a person is doing on a
   screen.
3. **No persona, interview, job-to-be-done or task-flow appears anywhere in the
   design record.** [12](12-COMPETITIVE-ANALYSIS.md) is a strong *strategy*
   teardown — extension models, reporting philosophies, where to differ — and its
   own last section is "Open questions to resolve with research". They were never
   resolved.

The consequence is structural, not cosmetic. With no layer between "build a work
tracker" and "implement the API", the surfaces were derived from the **resource
list**: a page for tasks, a page for the board, a page for reports, a settings
page each for roles, teams, workflow and tags — because there is an endpoint for
each. A product organised by its data model can be complete and still feel like a
rendering of a database, because no screen can answer *why am I here*.

That is the difference between this and the products it is measured against, and
no amount of restyling closes it.

## Method, and what it cannot do

This is **desk research**: the domain, the reference products, the repository's
own strategy analysis, and established interaction patterns. It is **not** user
interviews, diary studies, or usage analytics, because this project has no users
yet and inventing quotes would be worse than having none.

So every claim below is marked:

- **[Given]** — follows from the domain or from decisions already recorded here.
  Not in question.
- **[Pattern]** — an interaction pattern the category has converged on, which is
  evidence about user expectation rather than proof about our users.
- **[Assumption]** — a judgement I am making to move forward. Each one is listed
  again in §Open questions with what would settle it.

An assumption acted on and labelled is a decision. An assumption acted on and
unlabelled is what produced the current product.

## The people

Five roles. A person can be several at once — the roles are *hats*, not accounts,
which is why the permission model grants at scope rather than by title
([04](04-RBAC-AND-AUTHORIZATION.md)). **[Given]**

| Role | What they own | What they are measured on | What they fear |
| --- | --- | --- | --- |
| **Doer** — engineer, designer, writer | Their own items | Finishing the right thing | Working on something that turned out not to matter; being blocked and nobody noticing |
| **Lead** — tech lead, EM, PM | A team's flow | Predictability; nothing stuck | Being surprised. Finding out late |
| **Reporter** — support, sales, ops, another team | The things they raised | Getting an answer | Their item vanishing into a backlog with no signal |
| **Admin** — workspace owner, ops | Configuration and access | Nothing broken, nobody over-permitted | Being the bottleneck; granting too much by accident |
| **Integrator** — plugin or API author | A contract | Their integration not breaking | Silent contract drift |

The **Doer is the overwhelming majority of sessions** and the Lead is the
majority of *anxiety*. **[Assumption]** — but a safe one: every product in the
category optimises the doer's day and sells to the lead's fear.

## The moments — when a work tracker is actually opened

Nobody opens a work tracker to "view tasks". They open it at a small number of
recurring moments, and each has a question and a time budget. **[Pattern]**

| # | Moment | Who | The question | Budget | Ends in |
| --- | --- | --- | --- | --- | --- |
| M1 | **Starting the day** | Doer | "What is mine, and what changed while I was away?" | ~15 s | Opening one item |
| M2 | **Finishing something** | Doer | "Where does this go now?" | ~5 s | A status change |
| M3 | **Interrupted** — someone asks | Doer, Reporter | "Where is WR-125?" | ~10 s | Reading one item, or answering in chat |
| M4 | **Blocked** | Doer | "Who do I need, and does anyone know I am stuck?" | ~20 s | A comment, or a blocker link |
| M5 | **Capturing** | Anyone | "Write it down before I forget" | ~10 s | An item that exists, even if incomplete |
| M6 | **Standup / sync** | Lead + team | "What moved, what is stuck, what is at risk?" | ~2 min | Talking, not clicking |
| M7 | **Planning** | Lead | "What is next, and is it too much?" | ~20 min | Ordering and assigning |
| M8 | **Answering upward** | Lead | "Will it land, and what is in the way?" | ~5 min | A sentence to a stakeholder |
| M9 | **Onboarding a person** | Admin | "Give them the right access, no more" | ~3 min | A grant |
| M10 | **Configuring** | Admin | "Make the tool match how we work" | rare, long | A workflow or role change |

Two things follow immediately.

**Most sessions are glances, not visits.** M1–M5 are seven of ten moments and
have budgets in seconds. A surface that answers a glance question but requires
scrolling, filtering or a second click *has failed the moment*, even if the
information was technically present. This is the mechanical reason the "no
scrolling" instruction keeps coming back: it is not a taste, it is the budget.

**The long moments are rare and can afford navigation.** Configuration (M10) and
planning (M7) are the only ones where a person will tolerate several steps. Those
are exactly the surfaces that currently look the most cared-for, and the glance
surfaces are the ones that look like a database.

## The jobs

Written as jobs-to-be-done, ranked by frequency × pain. The rank is what should
drive build order and screen real estate; it currently drives neither.

| Rank | Job | Moment | Frequency |
| --- | --- | --- | --- |
| J1 | Know what I should be doing right now | M1 | Every session |
| J2 | Record that something moved | M2 | Many per day |
| J3 | Find one specific item fast | M3 | Many per day |
| J4 | Say something about an item, to a person | M4 | Several per day |
| J5 | Capture a new item in one breath | M5 | Several per day |
| J6 | See what the team's work is doing | M6, M8 | Daily |
| J7 | Decide what is next | M7 | Weekly |
| J8 | Give someone exactly the access they need | M9 | Weekly |
| J9 | Make the tool match our process | M10 | Rarely |

**The product's screens are ordered almost exactly backwards.** The most
elaborate, most complete surfaces are settings (J8, J9 — the two rarest jobs).
The first-ranked job, J1, has one view that shows a flat list and cannot say what
changed since yesterday.

## Surface briefs

The contract for each surface. **A screen that cannot state its job, its one
question and its first action does not have a reason to exist**, and that test
alone removes two of the current screens.

Format:

- **Job / question** — the one it serves. One, not several.
- **Above the fold** — what must be legible without scrolling, because the
  moment's budget does not allow it.
- **First action** — what the surface is trying to make easy.
- **Fails when** — the specific way this surface goes wrong.

---

### S1 — Home (currently "My Work") · J1 · M1

- **Job / question** — "What is mine, and what changed while I was away?"
- **Above the fold** — what is assigned to me, ordered by *what needs deciding*
  and not by date; what changed since I last looked; what is blocked *on me*
  versus what I am blocked *by*.
- **First action** — open one item, or move one item.
- **Fails when** — it is a filtered task list. A filtered list answers "which
  items match a query"; the question here is "what deserves my attention", and
  those are different orderings. **The single largest gap in the product.**

The "what changed" half needs `activity` per task and a last-seen marker; both
exist (`GET /tasks/{id}/activity`, and the session), and nothing joins them.

### S2 — Item (task detail) · J3, J4 · M3, M4

- **Job / question** — "What is this, where is it, who has it, what is in the
  way, and what was said?"
- **Above the fold** — title, status, assignee, the blocker if any, and the
  latest comment. All five, with no scroll. **[Pattern]** — this is the one
  screen every competitor packs densely for exactly this reason.
- **First action** — comment, or transition.
- **Fails when** — the answer requires scrolling past description, subtasks and
  blockers to reach the conversation. Which is what it does today: the comment
  thread is below three sections.

### S3 — Board · J2, J6 · M2, M6

- **Job / question** — "Where is everything, and what is stuck?"
- **Above the fold** — every column, with counts; anything blocked or overdue
  marked *without* opening it.
- **First action** — drag one card.
- **Fails when** — it is used as the answer to J1. A board is a *team* surface;
  an individual's "what is mine" is a different question and gets its own screen.

### S4 — List · J7 · M7
- **Job / question** — "What is next, and is it too much?"
- **Above the fold** — enough rows to compare, with the fields you are ordering
  by visible: priority, estimate, assignee, due.
- **First action** — reorder, bulk-change, or assign.
- **Fails when** — it is a table of everything with no ordering opinion. Today it
  shows type, key, title, priority, due and updated — three of which nobody
  plans by — and cannot select more than one row, so `POST /tasks/bulk` has no UI.

### S5 — Team view · J6, J8 · M6, M8
- **Job / question** — "How is the team doing, and who is overloaded?"
- **Above the fold** — per person: in-flight count, blocked count, overdue count.
- **First action** — reassign, or open a blocked item.
- **Does not exist yet.** It is the surface the Lead's anxiety actually needs,
  and the one that would make TaskForge worth switching to.

### S6 — Administration · J8, J9 · M9, M10
- **Job / question** — "Give this person the right access" / "make the tool match
  our process".
- **Above the fold** — the current state, before any control to change it.
- **First action** — grant, or edit.
- **Fails when** — it is organised by *table* rather than by task. "Onboard
  Sarah" today means: Members → invite; Teams → add; Roles → grant. Three
  screens, one job. **[Pattern]** — the reference products offer the job as one
  flow and keep the tables underneath for when you need them.

### S7 — Reports · J6 · M8
- **Job / question** — "Will it land, and what is in the way?"
- **Above the fold** — a trend and an exception list, not a chart menu.
- **First action** — copy a sentence into a status update.
- **Not built**, and correctly deferred — but it should be built to answer *that
  sentence*, not to render every metric the data model permits.

---

Two current screens fail the test outright:

- **"All tasks" with no project scope** serves no moment. Nobody's question is
  "show me every task in the workspace". It exists because `GET /tasks` exists.
  It should become **Search results** — the destination of J3 — which is a real
  moment with a real question.
- **Tags settings** is a table for a vocabulary of, typically, under twenty
  items, none of which anyone visits on purpose. It belongs inside the tag picker
  where the need arises, not as a destination in the navigation.

## What this explains about the current product

Every complaint made about it is predicted by the briefs above.

| Symptom | Cause |
| --- | --- |
| "Feels like a school project" | Screens are named after resources, so none states a purpose. The eye meets a toolbar first because no surface has a subject. |
| "I hate scrollers" | Glance surfaces (S1, S2) are laid out like documents. The budget is seconds; the layout assumes minutes. |
| "No research" | Correct. Screen inventory equals endpoint inventory. |
| Settings look more finished than the work surfaces | Configuration is the one job whose shape *is* a table, so building from the data model accidentally suited it. |
| Bulk endpoint with no UI | The list was built to display rows, not to plan — and planning is what needs selection. |

## What changes, in order

Driven by job rank, not by what is easiest to build next.

1. **Home becomes a real answer to J1** — grouped by what needs deciding, with a
   "changed since you were here" section. Highest-frequency job, worst current
   surface.
2. **The item surface is re-laid out for the glance** — status, assignee, blocker
   and the last comment above the fold; description and history below.
3. **Selection and bulk actions on the list** — J7, and it makes an endpoint that
   already exists reachable.
4. **The team view** — the missing surface, and the strongest reason to switch.
5. **Administration reorganised by job** — "add someone", "change what a role
   can do", with the tables kept underneath.
6. **"All tasks" becomes Search results**, and tags leave the navigation.

Each is a change in *what the screen is for*, and each brings a layout change
with it. None of them is a restyle.

## Open questions — the ones that need real users

Each is an **[Assumption]** above that I could not settle from the desk.

1. **Is "what changed since I was here" the right second half of Home**, or do
   people want "what is due"? Both are defensible; they produce different
   screens. *Settle with:* five doers, asked what they check first and why.
2. **Do leads want a team view, or do they want a digest they never open the
   product for?** *Settle with:* three leads, asked how they currently answer M8
   — if the answer is "I ask in chat", the product should send, not display.
3. **Is the board or the list the primary planning surface** for teams of this
   size? The category is split. *Settle with:* observing one planning session.
4. **How often is a second project actually open at once?** It decides whether
   the project is navigation (as now) or a filter. *Settle with:* usage data,
   once there is any.
5. **What do people abandon trackers for?** — already asked in
   [12](12-COMPETITIVE-ANALYSIS.md) and still unanswered. It decides what is
   first-class rather than a plugin.

Until each is settled, the surfaces built from them carry the assumption in their
module documentation, so the next person can see what was guessed.
