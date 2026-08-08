# 04 — RBAC & Authorization

**This is the most load-bearing design note in the set.** Everything else can be
changed incrementally; the permission model cannot. It is fixed here, before code
(ADR-004).

The old drafts listed scopes and permission names but never said **how a decision
is reached**. That gap is closed below: a total, deterministic, explainable
function.

## The decision function

```
authorize(actor, permission, resource) -> Allow | Deny(reason)
```

Total (never "maybe"), deterministic (same inputs ⇒ same answer), and
**explainable** — every `Deny` names the reason, and every `Allow` can name the
grant that produced it. The `/permissions/explain` endpoint returns exactly this
([05](05-API-SPEC.md)).

## Vocabulary

| Term | Meaning |
| --- | --- |
| **Permission** | A stable string `resource.action`, e.g. `task.close`. The full set is a closed registry, versioned with the API. |
| **Role** | A named set of permissions. Built-in templates + admin-defined custom roles. Roles hold **no** scope of their own. |
| **Principal** | Something a role can be assigned to: a **user**, a **team**, or a **service account**. |
| **Scope** | Where an assignment applies: `WORKSPACE`, `TEAM`, `PROJECT`, `ENVIRONMENT`. |
| **Grant** | One row: `(principal, role, scope_type, scope_id)`. The only source of authority. |
| **Constraint** | An optional narrowing predicate on a grant, e.g. `assignee_is_actor`. |

## The scope containment chain

Scopes form a strict containment hierarchy. A grant applies to a resource if the
grant's scope **contains** that resource:

```
WORKSPACE
    ├── TEAM ────────┐
    └── PROJECT ◀────┘        (a project may belong to a team)
            └── ENVIRONMENT
                    └── (tasks live in a project, optionally tagged to an environment)
```

For a task in project `P` (in team `T`, workspace `W`, environment `E`), the
**applicable scope set** is `{W, T, P, E}` — every ancestor, plus the resource's
own scope. A grant at any of them contributes.

> **`TASK` scope is not implemented in v1.** The old draft reserved it for
> "exceptional sharing." Per-task grants multiply the grant table by the task
> count and make the resolver unbounded. Deferred to a plugin-provided share link
> with its own token, which is a different mechanism. Recorded as ADR-005.

## Resolution: additive union, no deny rules

**The rule, in one sentence:** an actor's effective permissions on a resource are
the **union** of the permissions from every grant whose scope contains that
resource and whose principal includes the actor.

```
effective(actor, resource):
    principals = {actor} ∪ teams_of(actor) ∪ {service_account if acting as one}
    scopes     = ancestors_of(resource) ∪ {scope_of(resource)}

    grants     = { g ∈ role_assignment
                 | g.principal ∈ principals
                 ∧ (g.scope_type, g.scope_id) ∈ scopes
                 ∧ g.workspace_id = resource.workspace_id }

    return ⋃ { (p, g.constraints) | g ∈ grants, p ∈ permissions_of(g.role) }
```

### There are no deny rules. This is deliberate.

The alternative — allow and deny grants with precedence — is where permission
systems go to die. It forces an ordering question at every level (does a
project-level deny beat a workspace-level allow? does a team deny beat a direct
user allow?), and the answer is never intuitive to the admin who has to predict
it. Jira's permission schemes, AWS IAM, and Kubernetes RBAC land on three
different answers; only Kubernetes (purely additive) is routinely described as
comprehensible.

TaskForge is additive:

- More grants can only ever mean **more** access.
- To reduce access, **remove a grant** — the mental model is "who did I give this
  to," which admins can actually audit.
- The `/permissions/explain` answer is always a short list of contributing grants,
  never a precedence trace.

**Cost, stated honestly:** you cannot express "Member everywhere *except* this one
project." You express it by not granting Member at workspace scope and granting it
per project instead, or by using a team. Real deployments that need workspace-wide
baseline access with a single exclusion should make the sensitive project
**private** — visibility handles exclusion, permissions handle capability. This is
the trade recorded in ADR-004; revisiting it requires a superseding ADR.

### Most-specific-wins applies to *constraints*, not to grants

Constraints narrow, but only within their own grant. The combining rule:

```
allows(actor, permission, resource):
    contributing = { (p, c) ∈ effective(actor, resource) | p = permission }
    if contributing is empty:              return Deny(NoGrant)
    if any (p, c) with c = ∅:              return Allow          # unconstrained grant wins
    if any (p, c) with satisfied(c, actor, resource):
                                            return Allow
    return Deny(ConstraintUnsatisfied)
```

So an **unconstrained grant always beats a constrained one**. If a user is
"Member (may edit only own tasks)" on the workspace and "Project Manager
(unconstrained)" on one project, they may edit anything in that project. That is
the additive model behaving consistently — a constraint is a property of a grant,
never a restriction on other grants.

### Constraint set (v1 — closed)

Kept deliberately small. Each is a pure predicate over `(actor, resource)`, all
inputs already loaded, no extra queries.

| Constraint | Satisfied when |
| --- | --- |
| `assignee_is_actor` | actor ∈ task.assignees |
| `reporter_is_actor` | task.reporter = actor |
| `is_project_member` | actor has a `project_membership` row for the task's project |
| `environment_in` | task.environment ∈ grant.environment_ids |
| `not_external` | actor's workspace membership type ≠ `GUEST` |

Adding a constraint type is an ADR trigger ([11](11-DESIGN-FIRST-PROCESS.md)) —
this list is the thing that grows into an unreadable policy engine if left
unguarded.

## Visibility vs permission

Two separate questions, often conflated:

- **Visibility** — *can this actor see the project exists?* A property of the
  project: `PRIVATE` (members only) / `TEAM` / `WORKSPACE`.
- **Permission** — *can this actor do X here?* Resolved as above.

Visibility is evaluated **first** and produces an implicit read grant:

```
visible(actor, project):
    project.visibility = WORKSPACE                     → yes (any workspace member)
    project.visibility = TEAM   ∧ actor ∈ project.team → yes
    actor has a project_membership row                 → yes
    actor holds any grant scoped to this project       → yes
    otherwise                                          → no  (404, not 403)
```

An invisible project returns **404, not 403** — a 403 leaks that the project
exists. Once visible, the actor still holds only the permissions their grants
give them; visibility alone confers `project.read` and `task.read`, nothing more.

## Privilege escalation controls

The rules that stop RBAC from being a self-service root exploit:

1. **Grant ceiling.** An actor may assign role `R` at scope `S` only if
   `permissions_of(R) ⊆ effective(actor, S)`. You cannot grant what you do not
   hold. Checked at assignment time *and* re-checked on role edit — editing a
   role you granted cannot smuggle in new permissions.
2. **Scope ceiling.** Assigning at scope `S` requires the scope-appropriate
   assign permission held **at or above** `S`. A project manager holding
   `project.role.assign` at project scope cannot create workspace grants.
3. **Role editing is workspace-scoped.** `role.manage` exists only at workspace
   scope. Project managers assign roles; they do not author them.
4. **Last-owner protection.** The final grant carrying `workspace.owner` cannot be
   removed or downgraded. Enforced as a database constraint check inside the
   transaction, not just in application code.
5. **Self-elevation block.** An actor may not add permissions to their own
   effective set at their own scope — assignments where `principal = actor` and
   the role exceeds the actor's current set are rejected, even if the actor holds
   the assign permission.
6. **Plugin ceiling.** A plugin's effective permissions are the **intersection**
   of its granted scopes and the installing admin's permissions at install time.
   Consent is explicit and recorded ([34](34-PLUGIN-AND-EXTENSION-ARCHITECTURE.md)).
7. **Everything is audited.** Every grant, revoke, role edit, and consent writes
   an `audit_event` with before/after ([25](25-EVENTS-OUTBOX-AND-AUDIT.md)).

## Built-in role templates

Templates are **cloneable starting points**, not special-cased code. Cloning
produces an ordinary custom role; nothing in the resolver knows a role is
"built-in."

| Role | Shape |
| --- | --- |
| **Owner** | Everything, including `workspace.delete` and billing. Last one is protected. |
| **Administrator** | Everything except workspace deletion/transfer. |
| **Project Manager** | Full control within scoped projects: members, workflow, roles (under both ceilings), all task actions. |
| **Member** | Create/update/comment/transition tasks; read the project. No config, no role assignment. |
| **Guest** | Read + comment on projects they are explicitly a member of. Carries `not_external` exclusions elsewhere. |

## Caching, and why it is never the authority

Resolution touches `role_assignment`, `role_permission`, and team membership.
Uncached, that is 3 joins per request — acceptable, but not per *object* in a
list of 200.

**The cache:**

- Key: `(workspace_id, actor_id, project_id, authz_epoch)`.
- Value: the effective permission set + constraints for that project.
- TTL: 60 s, in-process (moka) with Redis as an optional shared tier.

**`authz_epoch`** is a per-workspace counter bumped by any grant, role, team
membership, or project membership change, in the same transaction as the change.
A stale epoch cannot be read: the key simply misses. This gives immediate
invalidation without a fan-out invalidation message, and it is why the TTL can be
generous.

**Non-negotiable rules:**

- Mutations re-resolve against the database inside their transaction. The cache
  serves reads and UI affordance computation only.
- Long-lived SSE subscriptions re-check membership on every epoch bump, not just
  at connect ([05](05-API-SPEC.md)).
- The cache is per workspace and never crosses tenants
  ([32](32-TENANCY-AND-ISOLATION.md)).

## The list problem, and how it is solved

Authorizing 200 tasks in a board one-by-one is 200 resolutions. TaskForge avoids
it structurally: **task-level permissions never vary within a project** (there is
no `TASK` scope, per ADR-005). Therefore:

1. Resolve the actor's accessible project set **once** — `accessible_projects()`,
   cached per epoch.
2. Filter the query by `project_id = ANY($accessible)`, which is indexed
   ([26](26-SEARCH-INDEXING-AND-QUERY.md)).
3. Apply per-task constraints (`assignee_is_actor` etc.) as **SQL predicates**,
   not as a post-filter — so pagination counts stay correct.

Post-filtering an authorized page is a bug, not an optimization: it silently
shrinks pages and breaks cursors. The filter belongs in the query.

## Endpoints

| Endpoint | Purpose |
| --- | --- |
| `GET /api/v1/permissions/effective?project_id=` | The actor's permission set — what the client uses to render affordances. |
| `POST /api/v1/permissions/explain` | `{actor, permission, resource}` → decision **plus contributing grants or the deny reason**. The admin's debugging tool and the simulator's backend. |
| `POST /api/v1/roles`, `PATCH /api/v1/roles/{id}` | Role authoring (workspace scope only). |
| `POST /api/v1/role-assignments` | Grant creation, subject to both ceilings. |

`/permissions/explain` is not a nice-to-have. "Why can't I close this?" is the
single most common support question in every tracker, and the additive model is
what makes the answer short enough to show a user.

## Acceptance gates

Authorization ships with tests, not confidence:

- **Matrix test** — every permission × every built-in role × every scope, as a
  golden table. Any resolver change that shifts a cell must shift it in the
  fixture too.
- **Escalation suite** — one test per control above, each *attempting* the
  exploit and asserting rejection.
- **Additivity property test** — for random grant sets: adding a grant never
  removes a permission. This is the invariant the whole model rests on.
- **Isolation property test** — no grant in workspace A ever affects a decision
  in workspace B.
- **No-N+1 test** — a 200-task board issues exactly one authorization resolution.
- **404-not-403 test** — invisible projects are indistinguishable from absent
  ones across every endpoint.

## Alternatives considered

| Option | Why not |
| --- | --- |
| **Allow + deny with precedence** (Jira schemes, IAM) | Expressive, but the precedence rules are not predictable by the admins who configure them. The support cost is permanent. Rejected — see the honest cost note above. |
| **ReBAC / Zanzibar** (SpiceDB, OpenFGA) | Genuinely better for deep object graphs and per-object sharing. TaskForge's graph is 4 levels deep and homogeneous within a project; a relationship service adds an operational dependency and a network hop to every request for expressiveness we deliberately do not want. Revisit only if per-task sharing becomes a real requirement. |
| **Postgres RLS as the authorization mechanism** | RLS cannot express role/constraint logic without policy functions that are hard to test and impossible to explain to a user. Kept as a **tenancy backstop** ([32](32-TENANCY-AND-ISOLATION.md)), not as the authorization engine. |
| **Permissions baked into roles at assignment time** (denormalized) | Faster reads, but a role edit would need to rewrite every assignment, and audit becomes ambiguous. The `authz_epoch` cache gets the same speed without the write amplification. |

## ADRs triggered

- **ADR-004** — Additive-union RBAC with no deny rules.
- **ADR-005** — `TASK` scope excluded from v1.
- **ADR-012** — `authz_epoch` cache invalidation.
