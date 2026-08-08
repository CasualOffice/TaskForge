# 27 — Filter Grammar & Saved Views

The typed grammar every list, board, search, saved view, and automation condition
is expressed in. One grammar, used everywhere — so a filter a user builds in the
UI is the same object an automation rule evaluates and a plugin receives.

The old drafts listed `saved_view` as a table and never defined what it stores.
This is that definition.

## Design constraints

1. **Closed field set.** Only the fields in [26](26-SEARCH-INDEXING-AND-QUERY.md)
   are filterable, and each has an index. An unknown field is `400`, never a slow
   query (ADR-011).
2. **Statically typed.** Each field has a type; each type permits specific
   operators. `due_at = "urgent"` is rejected at parse time, not at the database.
3. **Bounded.** Max 32 clauses, max depth 4 ([21](21-API-LIMITS-AND-QUOTAS.md)) —
   so the worst case a user can construct is a known query cost.
4. **Two surfaces, one AST.** A URL query string for links and a JSON tree for
   storage. Both parse to the same structure; either can be rendered from it.
5. **No raw SQL, ever.** The AST compiles to parameterized SQL through a
   whitelist. There is no path from user input to a SQL fragment.

## The AST

```jsonc
{
  "op": "and",
  "clauses": [
    { "field": "state",    "op": "in",     "value": ["ACTIVE", "PLANNED"] },
    { "field": "assignee", "op": "eq",     "value": "@me" },
    { "field": "due_at",   "op": "before", "value": "+7d" },
    {
      "op": "or",
      "clauses": [
        { "field": "priority", "op": "gte", "value": "HIGH" },
        { "field": "tag",      "op": "in",  "value": ["security"] }
      ]
    }
  ]
}
```

Two node kinds only: a **group** (`and` | `or` | `not`) and a **clause**
(`field`, `op`, `value`). Deliberately not a general expression language — there
are no functions, no arithmetic, no field-to-field comparison. Every one of those
would break the index guarantee.

## Fields and their operators

| Field | Type | Operators |
| --- | --- | --- |
| `project` | id | `eq` `in` `not_in` |
| `status` | id | `eq` `in` `not_in` |
| `state` | enum | `eq` `in` `not_in` |
| `type` | enum | `eq` `in` `not_in` |
| `priority` | ordered enum | `eq` `in` `gt` `gte` `lt` `lte` |
| `assignee` | id | `eq` `in` `is_empty` `is_not_empty` |
| `reporter` | id | `eq` `in` |
| `tag` | id | `in` `not_in` `is_empty` |
| `milestone` | id | `eq` `in` `is_empty` |
| `environment` | id | `eq` `in` `is_empty` |
| `parent` | id | `eq` `is_empty` `is_not_empty` |
| `created_at` | datetime | `before` `after` `between` |
| `updated_at` | datetime | `before` `after` `between` |
| `due_at` | datetime | `before` `after` `between` `is_empty` |
| `key` | text | `eq` `starts_with` |
| `title` | text | `contains` |
| `q` | fulltext | `matches` (the whole-document search) |
| `is_blocked` | boolean | `eq` — derived, backed by `task_dependency_rev_ix` |
| `archived` | boolean | `eq` — defaults to `false` when unspecified |

`priority` supports ordered comparison because it is a Postgres enum with
semantic ordering ([22](22-DATABASE-SCHEMA.md)) — `gte HIGH` is an index range
scan, not a `CASE` expression.

**Adding a field** requires: an entry here, an index in
[26](26-SEARCH-INDEXING-AND-QUERY.md), an `EXPLAIN` assertion, and a UI control.
All four in one change, or none.

## Symbolic values

Resolved at evaluation, so saved views stay correct as context changes:

| Symbol | Resolves to |
| --- | --- |
| `@me` | the requesting actor |
| `@my_teams` | teams the actor belongs to |
| `@unassigned` | sugar for `assignee is_empty` |
| `+7d` `-30d` `+1w` `-3mo` | relative to now, at evaluation |
| `@today` `@tomorrow` `@start_of_week` | in the **actor's** timezone |
| `@current_milestone` | the project's nearest incomplete milestone |

`@me` is what makes one saved view — "My overdue work" — correct for every user
who opens it. A view that hardcoded a user id would be shareable but wrong.

**Timezone is the actor's, not the server's.** `due before @today` must mean the
same thing to someone in Auckland and someone in Los Angeles. Server-local
date boundaries are a classic and extremely confusing bug.

## URL form

For shareable links and the browser address bar:

```
?state=ACTIVE,PLANNED&assignee=@me&due_at=<+7d&priority=>=HIGH&sort=-due_at
```

| Syntax | Meaning |
| --- | --- |
| `field=a,b` | `in [a, b]` |
| `field=!a` | `not_in [a]` |
| `field=<x` / `field=>x` | `before` / `after`, or `lt` / `gt` |
| `field=>=x` | `gte` |
| `field=x..y` | `between` |
| `field=` | `is_empty` |
| `sort=-due_at,key` | descending `due_at`, then ascending `key` |

The URL form expresses only flat `AND`. Nested groups require the JSON form —
a deliberate limit that keeps URLs readable, with the UI switching to a stored
view when a user builds something nested.

## Compilation

```
URL string ──┐
             ├──▶ AST ──▶ validate ──▶ compile ──▶ parameterized SQL
JSON tree ───┘            (fields,     (whitelist
                           types,       + index
                           ops, limits) hints)
```

The compiler emits only `$1`-style parameters and identifiers from a static map.
There is no string interpolation of user data anywhere in the path — the property
test asserts this over random ASTs including hostile input.

Compiled shape:

```sql
SELECT t.* FROM task t
 WHERE t.workspace_id = $1
   AND t.project_id = ANY($2)          -- permission filter, always injected
   AND t.deleted_at IS NULL
   AND t.archived_at IS NULL           -- unless archived=true
   AND t.state = ANY($3)
   AND EXISTS (SELECT 1 FROM task_assignee a
                WHERE a.task_id = t.id AND a.user_id = $4)
   AND t.due_at < $5
 ORDER BY t.due_at ASC, t.id ASC
 LIMIT 51;
```

**The permission filter is injected by the compiler, not supplied by the caller.**
It is structurally impossible to compile a filter that omits it — the compiler's
signature requires an `AuthorizedProjectSet`. A missing tenant filter cannot be a
code-review oversight ([04](04-RBAC-AND-AUTHORIZATION.md)).

Many-to-many fields (`assignee`, `tag`) compile to `EXISTS`, not `JOIN`, so a
task matching two tags appears once without a `DISTINCT` — which would break
cursor pagination.

## Saved views

A saved view is a named filter + sort + layout:

```json
{
  "name": "My overdue work",
  "filter": { "op": "and", "clauses": [ ... ] },
  "sort": [ { "field": "due_at", "dir": "asc" } ],
  "layout": "LIST",
  "shared": false
}
```

- **Scope** — workspace-wide (`project_id` null) or project-scoped.
- **Sharing** — private by default. A shared view is *visible* to project members,
  but **executes with the viewer's permissions**: two people opening the same
  shared view see different rows, and neither sees anything they could not
  otherwise. A saved view is never a permission-bypass channel.
- **Ownership** — only the owner or a `project.manage` holder may edit or delete.
  Deleting a view others use is warned, with a usage count.
- **Layout** — `LIST` | `BOARD` | `TABLE`. Board layout additionally stores its
  grouping field.

### Built-in views

Shipped, non-deletable, expressed in the same grammar — proof the grammar is
sufficient:

| View | Filter |
| --- | --- |
| My Work · Today | `assignee=@me AND state in (PLANNED,ACTIVE) AND due_at <= @today` |
| My Work · Overdue | `assignee=@me AND state not in (COMPLETED,CANCELED) AND due_at < @today` |
| My Work · Upcoming | `assignee=@me AND due_at between @tomorrow..+14d` |
| My Work · Blocked | `assignee=@me AND is_blocked=true` |
| My Work · Recently completed | `assignee=@me AND state=COMPLETED AND updated_at > -7d` |
| Reported by me | `reporter=@me AND state not in (COMPLETED,CANCELED)` |
| Unassigned | `assignee is_empty AND state in (BACKLOG,PLANNED)` |

If a built-in view needed a capability the grammar lacks, that is the signal the
grammar is under-specified — and it is why they are defined this way rather than
hand-written SQL.

## Reuse in automations

Automation rule conditions use the **same AST** ([36](36-AUTOMATION-RULES-DESIGN.md)),
evaluated in-memory against a changed task rather than compiled to SQL. One
grammar, two evaluators, identical semantics — so a user who builds a filter can
build a rule condition without learning anything new.

The in-memory evaluator is property-tested against the SQL compiler: for random
ASTs and random tasks, both must agree. Divergence between "what the filter shows"
and "what the rule matches" would be a permanent source of confusion.

## Acceptance gates

- **No-injection property test** — random and adversarial ASTs; assert the
  emitted SQL contains no user-derived string outside a bind parameter.
- **Compiler/evaluator agreement** — random AST × random task: SQL result and
  in-memory result agree.
- **Permission-filter test** — no compiled query lacks the project filter, checked
  by inspecting emitted SQL for every field/operator combination.
- **`EXPLAIN` suite** — every field × every operator produces an index scan
  ([26](26-SEARCH-INDEXING-AND-QUERY.md)).
- **Round-trip test** — URL → AST → URL is stable; AST → JSON → AST is identity.
- **Timezone test** — `@today` evaluated for actors in three timezones yields
  three different boundaries, each correct.
- **Limit test** — 33 clauses, or depth 5, is rejected with the specific code.
