# 03 — Domain Model

The entities, their invariants, and their lifecycles. The physical schema is
[22](22-DATABASE-SCHEMA.md); this is the meaning behind it.

Held to the simplicity contract ([01](01-ORD.md)): **eleven nouns a user must
learn.** Everything else is a property of one of them.

## The eleven

| Noun | One line |
| --- | --- |
| **Workspace** | The tenant. Everything belongs to exactly one. |
| **User** | A person. Belongs to many workspaces. |
| **Team** | A named group of users; a principal for permissions and a home for projects. |
| **Project** | The collaboration boundary. Owns tasks, a workflow, and its members. |
| **Environment** | An optional context within a project (QA, Staging, Region-EU). |
| **Task** | The universal work item. Everything trackable is one. |
| **Tag** | A reusable label. Many-to-many with tasks. |
| **Milestone** | A dated target tasks belong to. |
| **Workflow** | Statuses and the transitions between them. |
| **Comment** | A message on a task. |
| **Attachment** | A file on a task. |

**Activity**, **audit**, **notification**, **saved view**, **role**, and
**plugin** exist in the schema but are not nouns users must *learn* — they are
either automatic (activity, audit, notification) or admin-only (role, plugin) or
a saved shape of something already understood (saved view).

## Workspace — the tenant boundary

Every row of tenant data carries `workspace_id`. There is no cross-workspace
query, join, cache key, object key, or job ([32](32-TENANCY-AND-ISOLATION.md)).

- A user reaches a workspace through a `workspace_membership` carrying a
  **membership type**: `MEMBER` or `GUEST`. Guests are external collaborators;
  the `not_external` constraint keys off this ([04](04-RBAC-AND-AUTHORIZATION.md)).
- A workspace always has at least one grant carrying `workspace.owner`. Enforced
  in-transaction, not by convention.
- Deleting a workspace is a **staged** operation: soft-delete → 30-day grace →
  hard delete of all tenant rows and object-store prefixes.

## Project — the collaboration boundary

- **Visibility** is `PRIVATE` | `TEAM` | `WORKSPACE`, and answers *who can see it
  exists*. It is not a permission ([04](04-RBAC-AND-AUTHORIZATION.md)).
- A project has a **key** (`WR`), unique per workspace, 2–10 uppercase chars. It
  is **immutable after creation** — task keys are printed in commit messages,
  chat, and tickets; rewriting them breaks external references. Renaming a
  project does not rename its key. (ADR-007)
- A project has exactly one workflow and zero or more environments.
- A project may belong to a team; that team then sits in the task's scope chain.
- Archiving hides a project from lists but preserves every URL and permission.
  Deletion is soft, with the same 30-day grace as workspaces.

## Task — the universal work item

The one model everything trackable uses. `type` distinguishes `TASK` `BUG`
`FEATURE` `INCIDENT` `REQUEST`; types differ in default fields and icon, never in
storage or capability.

### Identity

- `id` — UUIDv7. Sortable, globally unique, safe in URLs and logs.
- `key` — `{project.key}-{number}`, e.g. `WR-125`. The human handle.
- `number` — per-project, monotonic, **allocated inside the creating transaction**
  via `UPDATE project SET task_seq = task_seq + 1 RETURNING task_seq`. A row lock,
  not a sequence: sequences leak numbers on rollback, and users notice gaps in
  `WR-1, WR-2, WR-4`. Contention is bounded by tasks-per-project-per-second, which
  is not a real bottleneck. (ADR-008)
- Numbers are **never reused**, including after deletion.

### Fields

| Field | Notes |
| --- | --- |
| `title` | required, 1–512 chars |
| `description` | optional, markdown, 64 KB cap |
| `type` | one of the five |
| `priority` | `NONE` `LOW` `MEDIUM` `HIGH` `URGENT` — ordered enum, sortable |
| `status_id` | a status in the project's workflow |
| `state` | **derived** from status, denormalized for query ([23](23-WORKFLOW-AND-STATE-MACHINE.md)) |
| `reporter_id` | who filed it; immutable |
| assignees | 0..n via `task_assignee`, one optionally `is_primary` |
| `project_id` | required |
| `environment_id` | optional, **single-select** (ADR-010) |
| `milestone_id` | optional |
| `parent_id` | optional; subtasks |
| `start_at`, `due_at` | optional timestamps |
| tags | 0..n via `task_tag` |
| `position` | lexicographic board rank ([26](26-SEARCH-INDEXING-AND-QUERY.md)) |
| `version` | optimistic concurrency counter ([24](24-CONCURRENCY-AND-IDEMPOTENCY.md)) |
| `archived_at`, `deleted_at` | soft lifecycle |

### Decisions the old drafts left open, now settled

**Multiple assignees: yes**, with an optional primary. (ADR-010)
Single-assignee is cleaner but wrong for the pair-work, review, and incident
cases that trackers exist to handle; teams work around it with comments, which
destroys the "assigned to me" query. The primary flag preserves single-owner
accountability where a team wants it. `task_assignee` is a join table from day
one, so this is not a later migration.

**Environments: single-select.** (ADR-010)
Multi-select doubles the filter and reporting surface for a case ("this bug
affects QA and Staging") that is better modelled as linked tasks. If real usage
disproves this, single→multi is an additive migration; multi→single is not.

### Subtasks

`parent_id` gives one level of nesting that is *presentational*: a subtask is a
full task with its own key, status, assignees, and permissions.

- **Depth is capped at 1.** A subtask cannot have subtasks. Arbitrary trees force
  recursive queries into every list, board, and permission check, and users build
  unnavigable hierarchies. Deeper decomposition uses `task_dependency` or
  milestones.
- Parent status is **never** auto-derived from children. Rollup is displayed
  (`3/5 done`), never enforced — implicit status changes are the most confusing
  behaviour in every tracker that does it.
- Deleting a parent soft-deletes its children.

### Dependencies

`task_dependency (from_task_id, to_task_id, kind)` — the old drafts named the
table and never defined its semantics. Settled:

- **`kind` is `BLOCKS` only in v1.** `from` blocks `to`. Relates/duplicates are
  presentational links, not dependencies, and are deferred.
- **Cycles are rejected at write time** by a depth-limited reachability check
  (max 64 hops) inside the creating transaction. A cyclic dependency graph makes
  "is this unblocked?" undecidable and is always a data-entry mistake.
- **Dependencies gate transitions.** Moving a task into an `ACTIVE` or
  `COMPLETED` state while an incoming `BLOCKS` edge is unresolved is rejected —
  unless the workflow's transition sets `ignore_dependencies`, or the actor holds
  `task.dependency.override` (which records an audit event with a reason).
- Cross-project dependencies are allowed within a workspace; the blocking task
  shows as "restricted" if the viewer cannot see its project, never as its title.

## Workflow, status, state

The core of the model, specified in full in
[23](23-WORKFLOW-AND-STATE-MACHINE.md). The invariants that belong here:

- **State is a closed enum, forever**: `BACKLOG` `PLANNED` `ACTIVE` `COMPLETED`
  `CANCELED`. Adding one is a breaking API change.
- **Every status maps to exactly one state.** Statuses are renameable, reorderable
  and deletable; states are not.
- A task's `state` column is derived from its status and updated in the same
  statement. It exists so reports and My Work never join the workflow table.
- Deleting a status requires a **migration target** — the admin must say where
  in-flight tasks go. Statuses holding tasks cannot simply vanish. This was an
  open gap in the old drafts.

## Tags, milestones, environments

- **Tag** is scoped either to the workspace (`project_id IS NULL`) or to a
  project. Unique by `(workspace_id, project_id, name)`, case-insensitive. Tags
  are free-form on purpose — the governed alternative is a custom field.
- **Milestone** belongs to a project, has an optional `due_at`, and holds tasks.
  Completion is displayed, never enforced.
- **Environment** belongs to a project and is an ordered list. Deleting one
  requires a migration target, like statuses.

## Comment & attachment

- Comments are markdown, edit-tracked (`edited_at`), soft-deleted, and threaded
  one level via `parent_comment_id`. Mentions parse to user IDs at write time and
  fan out notifications after commit.
- Attachments are metadata rows; bytes live in object storage. A row is
  **invisible until `committed_at` is set** by the scan/commit handshake
  ([28](28-ATTACHMENT-PIPELINE.md)). An abandoned upload never appears.

## Activity vs audit

Two streams, deliberately separate ([25](25-EVENTS-OUTBOX-AND-AUDIT.md)):

| | Activity | Audit |
| --- | --- | --- |
| Audience | users | security / compliance |
| Content | "Sarah moved this to In Progress" | actor, IP, user agent, request id, session, before/after |
| Access | anyone with `task.history.read` | `audit.read`, workspace scope |
| Retention | project lifetime | policy-set, default 400 days |
| Written | on domain change | on domain change **and** on auth, permission, plugin, and admin events |

Both are append-only. There is no update or delete path in the application at all
— not "we don't do it," but no code exists that can.

## Lifecycle: archive vs soft-delete vs hard-delete

Three distinct states, previously conflated:

| | Archive | Soft delete | Hard delete |
| --- | --- | --- | --- |
| Visible in lists | no | no | n/a |
| Reachable by URL/key | **yes** | no (410 Gone) | no (404) |
| Counted in reports | optional | no | no |
| Reversible | yes, instantly | yes, within grace | no |
| Search | excluded by default | excluded | gone |

Soft-deleted records enter a **30-day grace period**, then a retention worker hard
deletes them. `deleted_at` predicates are in every hot index
([26](26-SEARCH-INDEXING-AND-QUERY.md)).

**User deletion** is different, because of GDPR: a deleted user's rows are
**anonymized in place**, not removed. The user account becomes a tombstone
(`Deleted User`, email nulled, PII scrubbed); authored tasks, comments, and
history keep their foreign keys. Deleting history to erase a person would destroy
the audit trail for everyone else. Full policy: [07](07-QUALITY-SECURITY-AND-COMPATIBILITY.md).

## Invariants (enforced, not documented)

Each is a database constraint, a transaction check, or a property test — never
just a rule in prose:

1. Every tenant row has a `workspace_id` matching its parents'.
2. A task's `status_id` belongs to its project's workflow.
3. A task's `state` equals its status's state.
4. A task's `environment_id` and `milestone_id` belong to its project.
5. `task.number` is unique per project and never reused.
6. A task's `parent_id` is in the same project and has no parent itself.
7. The dependency graph is acyclic.
8. Every workspace has ≥ 1 `workspace.owner` grant.
9. An assignee is a member of the task's project (unless admin-overridden).
10. `activity_event` and `audit_event` have no UPDATE or DELETE path.
11. An attachment without `committed_at` is invisible to every read path.

## ADRs triggered

- **ADR-007** — Immutable project keys.
- **ADR-008** — In-transaction task number allocation (no sequences).
- **ADR-010** — Multiple assignees with optional primary; single-select environment.
- **ADR-018** — Subtask depth capped at 1; no automatic status rollup.
- **ADR-019** — `BLOCKS`-only dependencies, cycle-rejected, transition-gating.
