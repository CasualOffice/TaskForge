# 17 — Glossary

The enforced vocabulary. Two rules:

1. **If a term is not here, users should not have to learn it.** Adding a
   user-facing noun is an ADR trigger ([11](11-DESIGN-FIRST-PROCESS.md)).
2. **One word per concept.** Not "issue" and "ticket" and "item" — **task**. Not
   "org" and "tenant" and "account" — **workspace**. Synonym drift in
   documentation becomes synonym drift in the UI, and then in the API.

## Product vocabulary (user-facing)

**Workspace** — the tenant boundary. Everything belongs to exactly one. A user
may belong to several.

**Project** — the collaboration boundary. Owns tasks, one workflow, its members,
its environments, and its milestones.

**Environment** — an optional context within a project: Development, QA, Staging,
Production, Region-EU. A task has at most one (ADR-010).

**Team** — a named group of users. A permission principal and an optional owner
of projects.

**Task** — the universal work item. Everything trackable is a task; `type`
distinguishes Task, Bug, Feature, Incident, Request.

**Subtask** — a task with a parent. One level deep, no more (ADR-018).

**Key** — a task's human handle, `{project key}-{number}`, e.g. `WR-125`.
Immutable, never reused (ADR-007, ADR-008).

**Status** — the configurable step a task is in: Todo, In Progress, Code Review.
Teams rename and rewire these freely.

**State** — the permanent semantic category a status maps to: `BACKLOG`,
`PLANNED`, `ACTIVE`, `COMPLETED`, `CANCELED`. **Five, forever.**

**Workflow** — a set of statuses plus the allowed transitions between them.

**Transition** — a permitted move from one status to another. The *only* way a
status changes.

**Dependency** — `A blocks B`. One kind in v1; cycles rejected (ADR-019).

**Tag** — a reusable free-form label, workspace- or project-scoped.

**Milestone** — a dated target that tasks belong to.

**Saved view** — a named filter + sort + layout ([27](27-FILTER-AND-SAVED-VIEW-DSL.md)).

**Activity** — the human-readable, append-only history of a task or project.

**Audit** — the security-grade record: who, from where, what changed, with
independent retention and access.

**Role** — a named set of permissions. Holds no scope of its own.

**Grant** *(user-facing: "role assignment")* — a role given to a principal at a
scope. **The only source of authority in the system.**

**Permission** — a single capability, `task.close`.

**Plugin** — an installed extension contributing to declared extension points.

**Automation** — a when/if/then rule over domain events.

## Technical vocabulary (not user-facing)

**Principal** — something a role can be assigned to: user, team, or service
account.

**Scope** — where a grant applies: `WORKSPACE`, `TEAM`, `PROJECT`, `ENVIRONMENT`.

**Constraint** — a narrowing predicate on a grant, e.g. `assignee_is_actor`.

**Effective permissions** — the union of all applicable grants for an actor on a
resource ([04](04-RBAC-AND-AUTHORIZATION.md)).

**`authz_epoch`** — a per-workspace counter, part of every permission cache key;
bumped in the same transaction as any grant or membership change (ADR-012).

**`WorkspaceScope`** — the capability type proving an authenticated tenant
context. Required by every repository method ([32](32-TENANCY-AND-ISOLATION.md)).

**Outbox** — the table domain events are written to, in the same transaction as
the change ([25](25-EVENTS-OUTBOX-AND-AUDIT.md)).

**Correlation ID** — the identifier tying a user action to every effect it caused,
across events, automations, and notifications.

**Extension point** — a declared, typed seam a plugin contributes to. The set is
closed and versioned (ADR-009).

**Plugin contract version** — the version a plugin pins, independent of the app
and the schema (ADR-015).

**Position / rank** — the lexicographic string ordering cards in a board column
(ADR-013).

**Cursor** — the opaque encoding of a sort key + id tiebreaker used for
pagination. Never parsed by clients.

**Projection** — a denormalized table maintained from events, e.g. `task_search`.

**Reference corpus** — the deterministic 2M-task dataset gates are measured
against ([30](30-PERFORMANCE-AND-CAPACITY-TARGETS.md)).

## Words we do not use

| Avoid | Use | Why |
| --- | --- | --- |
| issue, ticket, item, card, story | **task** | one model, one word |
| org, tenant, account, company | **workspace** | "tenant" is internal only |
| board column | **status** | a column *displays* a status |
| done / open / closed *as data* | **state** | `COMPLETED` is the contract; "done" is a status name |
| user group | **team** | one grouping concept |
| permission scheme, permission set | **role** | scheme indirection is what we rejected ([12](12-COMPETITIVE-ANALYSIS.md)) |
| ACL | **grant** | grants are additive, not lists |
| webhook *for the internal bus* | **outbox event** | a webhook is one consumer of one |
| app, add-on, integration, extension | **plugin** | one word for the one mechanism |
| epic | **milestone** or parent **task** | no new hierarchy noun (ADR-018) |
| sprint | *(not a concept in v1)* | agile ceremony belongs in a plugin |
| lossless, seamless, simply, just | — | vague words that hide unverified claims |

"Sprint" is the interesting omission: it is the most-requested concept that is
deliberately absent. Time-boxing is a plugin, because baking it into the core
would add a noun, a lifecycle, a report family, and a set of permissions to every
workspace that does not use it.
