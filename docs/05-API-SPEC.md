# 05 — API Specification

The REST + SSE contract. Versioned, cursor-paginated, optimistically concurrent,
and generated into OpenAPI from the Rust types so the document cannot drift from
the implementation.

## Principles

1. **Versioned in the path** — `/api/v1`. A breaking change means `/v2`, with
   both served during a deprecation window.
2. **Commands are POST to a named sub-resource** where the operation has rules —
   `POST /tasks/{id}/transitions`, not `PATCH {status}`. `PATCH` is reserved for
   plain field updates with no state machine behind them.
3. **Every mutation is authorized server-side.** The client's permission set
   drives affordances only ([04](04-RBAC-AND-AUTHORIZATION.md)).
4. **Cursor pagination everywhere.** No `offset`, ever ([26](26-SEARCH-INDEXING-AND-QUERY.md)).
5. **Optimistic concurrency on every mutable aggregate** via `If-Match`
   ([24](24-CONCURRENCY-AND-IDEMPOTENCY.md)).
6. **Structured errors** with stable codes ([20](20-ERROR-CODE-REGISTRY.md)).
7. **OpenAPI is generated**, not maintained — from `utoipa` annotations on the
   handler types. A drifted document is impossible.

## Conventions

| | |
| --- | --- |
| Content type | `application/json`, UTF-8 |
| Timestamps | RFC 3339, always UTC, always `Z` |
| IDs | UUIDv7 strings |
| Field case | `snake_case` (matches the domain and the SQL; no translation layer) |
| Unknown request fields | **rejected** with `400` — silently ignoring a typo'd field is how clients ship bugs that look like server bugs |
| Null vs absent in `PATCH` | absent = leave unchanged; `null` = clear |

That last row is the one most APIs get wrong. `PATCH {"due_at": null}` clears the
date; `PATCH {}` leaves it. Both are expressible, which JSON Merge Patch semantics
give us for free.

## Authentication

| Actor | Mechanism |
| --- | --- |
| Browser | `HttpOnly` `Secure` `SameSite=Lax` session cookie + CSRF token on unsafe methods |
| Service account / plugin | `Authorization: Bearer <token>` |
| Integration | scoped API token, per installation |

Details in [40](40-IDENTITY-AUTH-AND-SESSION.md). Every authenticated request
resolves to an `AuthContext { actor, workspace, scope }`, which is the only way a
`WorkspaceScope` is minted ([32](32-TENANCY-AND-ISOLATION.md)).

Workspace is determined by the path or an `X-Workspace-Id` header, and is
validated against membership on every request — never trusted from the client.

## Core endpoints

### Tasks

```
GET    /api/v1/tasks                       list/search (filter grammar, doc 27)
POST   /api/v1/projects/{id}/tasks         create        (Idempotency-Key)
GET    /api/v1/tasks/{id}                  read          (returns ETag)
PATCH  /api/v1/tasks/{id}                  update        (If-Match required)
DELETE /api/v1/tasks/{id}                  soft delete   (If-Match required)
POST   /api/v1/tasks/{id}/transitions      status change (If-Match required)
GET    /api/v1/tasks/{id}/assignees        who is on it
POST   /api/v1/tasks/{id}/assignees        assign
DELETE /api/v1/tasks/{id}/assignees/{uid}  unassign
POST   /api/v1/tasks/{id}/tags             tag
GET    /api/v1/tasks/{id}/dependencies     relations     (two named lists)
POST   /api/v1/tasks/{id}/dependencies     add dependency (cycle-checked)
DELETE /api/v1/tasks/{id}/dependencies/{other}  remove the edge, either way round
GET    /api/v1/tasks/{id}/activity         history       (cursor)
POST   /api/v1/tasks/{id}/comments         comment
GET    /api/v1/tasks/{id}/comments         thread        (cursor)
POST   /api/v1/tasks/{id}/attachments      begin upload  (doc 28)
POST   /api/v1/tasks/bulk                  bulk ops      (doc 24)

GET    /api/v1/tasks/{id}/custody           who has held it, where it has been (doc 45)
PUT    /api/v1/tasks/{id}/team              hand it to a team; clears assignees
POST   /api/v1/tasks/{id}/promotions        it reached an environment
POST   /api/v1/tasks/{id}/verifications     tested on an environment: PASS or FAIL
```

The custody four are one story told three ways ([45](45-DEVELOPMENT-LIFECYCLE-AND-CUSTODY.md)),
and their shapes follow from the process rather than from the tables:

- **Transfer is `PUT`**, because a task has exactly one owning team — but it is
  not idempotent in the log: Android → Backend → Android is two real events, and
  the bounce count is the number that exposes a broken process. Handing a task to
  the team that already owns it is `409`, so a retry cannot inflate that count.
- **Promotion is `POST` and deliberately not idempotent.** A second promotion to
  the same environment is a redeploy — a real event that a log swallowing
  duplicates would understate.
- **Verification is neither a field nor a status.** It is a verdict against the
  environment it was tested on, with evidence, and a task accumulates many:
  "failed twice on qa, then passed" is a sentence a status column cannot produce
  because a status only holds the latest value.
- **The read is one endpoint for three lists**, because they are one panel and
  always rendered together.

`PUT /tasks/{id}/environment` still exists and still carries `If-Match`; it now
writes a promotion row too, so the history is complete whichever door a task went
through.

The dependency **read** was not specified here and its shape is a choice, made
by C-008 and recorded rather than left to be inferred:

```json
{ "blocked_by": [ { "id", "key", "title", "state" } ], "blocks": [ … ] }
```

The **remove** takes no direction: at most one edge can join a pair — `A blocks
B` and `B blocks A` together are a cycle — so naming both ends identifies it, and
a direction parameter could only be a way to disagree with the graph. It does not
require the far end to be visible: `docs/03` shows an unreadable blocker as
`restricted` rather than hiding the edge, and demanding visibility would make
exactly those edges permanent while protecting nothing the caller cannot already
see. The authority is `task.update` on the task in the path, the same permission
that added it.

Two named lists rather than one array with a `direction` field, because the task
drawer renders them as two headed sections and a flat array makes every client
partition it again. `state` is included so a blocker that is already `COMPLETED`
can be struck through rather than shown as live. It is **not paginated**:
[21](21-API-LIMITS-AND-QUOTAS.md) bounds dependencies at 100 per task and that
bound is enforced on the write, so the whole set is one bounded response.

`GET /tasks` is one endpoint for lists, boards, My Work, saved views, and
full-text. They differ only in filter and sort — which is the point of having one
grammar ([27](27-FILTER-AND-SAVED-VIEW-DSL.md)).

### Projects, workflow, admin

```
GET    /api/v1/projects                    POST   /api/v1/projects
GET    /api/v1/projects/{id}               PATCH  /api/v1/projects/{id}
POST   /api/v1/projects/{id}/members       DELETE /api/v1/projects/{id}/members/{uid}
GET    /api/v1/projects/{id}/environments  POST   /api/v1/projects/{id}/environments

GET    /api/v1/workflows/{id}
POST   /api/v1/workflows/{id}/statuses
DELETE /api/v1/workflows/{id}/statuses/{sid}?migrate_to={sid}    ← doc 23
POST   /api/v1/workflows/{id}/transitions

GET    /api/v1/roles                       POST   /api/v1/roles
GET    /api/v1/role-assignments            POST   /api/v1/role-assignments
DELETE /api/v1/role-assignments/{id}
GET    /api/v1/teams/{id}/members          POST   /api/v1/teams/{id}/members
GET    /api/v1/permissions/effective?project_id=
POST   /api/v1/permissions/explain                                ← doc 04

GET    /api/v1/saved-views                 POST   /api/v1/saved-views
GET    /api/v1/notifications               POST   /api/v1/notifications/read
GET    /api/v1/audit-events                                       ← audit.read only
GET    /api/v1/plugins                     POST   /api/v1/plugins/{id}/install
```

## Pagination

```http
GET /api/v1/tasks?project=WR&state=ACTIVE&sort=-updated_at&limit=50
```

```json
{
  "data": [ { "id": "...", "key": "WR-125", "...": "..." } ],
  "page": {
    "next_cursor": "eyJrIjpbIjIwMjYtMDgtMDhUMTA6MTQ6MjJaIl0sImlkIjoiMDE5MiJ9",
    "has_more": true
  }
}
```

- `limit` default 50, max 100.
- `next_cursor` is opaque — clients must not parse it. Its internal shape is
  free to change.
- **No total count by default.** Counting matched rows is a second full scan of
  the match set; on a 2M-task workspace that is the expensive part of the
  request. `?include=count` opts in, and is capped at 10,000 with
  `"count_is_capped": true` rather than lying or hanging.

## Concurrency

```http
GET /api/v1/tasks/{id}            →  200, ETag: "7"

PATCH /api/v1/tasks/{id}
If-Match: "7"                     →  200, ETag: "8"
                                  →  409 if the task is now version 9
                                  →  428 if If-Match is missing
```

`428 Precondition Required` rather than silently accepting an unconditional write:
a client that forgets `If-Match` has a bug, and failing loudly in development is
better than losing a user's edit in production.

The `409` body carries the current representation and the changed fields, so the
client can show "Sarah changed status and assignee" and offer a merge
([24](24-CONCURRENCY-AND-IDEMPOTENCY.md)).

## Idempotency

```http
POST /api/v1/projects/{id}/tasks
Idempotency-Key: 018f2c...
```

Required on `POST` creates. A replay with the same key returns the original
response; the same key with a *different* body returns `422 TF-IDM-0002` — which
catches the client bug where a key is generated once and reused for a new task.
Keys are retained 24 hours.

## Errors

```json
{
  "error": {
    "code": "TF-WFL-0004",
    "message": "Required fields missing for transition to \"Done\"",
    "details": { "missing_fields": ["resolution", "fix_version"] },
    "request_id": "018f2c...",
    "docs": "https://docs.taskforge.dev/errors/TF-WFL-0004"
  }
}
```

Every error carries a stable code ([20](20-ERROR-CODE-REGISTRY.md)) and a
`request_id` the user can quote to support. `details` is machine-readable and
returns **all** violations at once, never the first one — a form that reveals
missing fields one round-trip at a time is a bad form.

| Status | Meaning |
| --- | --- |
| `400` | malformed / unknown field / bad filter |
| `401` | unauthenticated |
| `403` | authenticated, not permitted **on a resource you can see** |
| `404` | absent **or invisible** — never disambiguated ([04](04-RBAC-AND-AUTHORIZATION.md)) |
| `409` | version conflict |
| `410` | soft-deleted |
| `422` | valid syntax, violates a domain rule |
| `428` | `If-Match` required |
| `429` | rate limited (`Retry-After` always present) |
| `503` | shedding load (`Retry-After` always present) |

## Live updates (SSE)

```http
GET /api/v1/stream?project_id=...
Accept: text/event-stream
```

```
event: task.status.changed
id: 018f2c...
data: {"aggregate_id":"...","changes":{...}}
```

- **SSE, not WebSocket.** Traffic is overwhelmingly server→client; SSE is plain
  HTTP, survives proxies, and reconnects with `Last-Event-ID` natively. A
  WebSocket would be justified only by genuine client→server streaming, which no
  planned feature needs — and would require an ADR ([08](08-ADR-REGISTER.md)).
- **`Last-Event-ID` replay** on reconnect, bounded to 5 minutes / 1,000 events.
  Beyond that the client is told to refetch rather than being handed a partial
  history it would silently treat as complete.
- **Membership is revalidated on every `authz_epoch` change**, not only at
  connect. A revoked user's stream closes with `403` within one epoch bump — a
  long-lived stream is otherwise a permission-revocation hole
  ([04](04-RBAC-AND-AUTHORIZATION.md)).
- Events are **coalesced** per aggregate over a 100 ms window, so a rapid drag
  produces one update, not forty.
- Heartbeat comment every 30 s keeps intermediaries from closing idle streams.

## Bulk operations

```http
POST /api/v1/tasks/bulk
{ "operation": "transition", "task_ids": ["..."], "to_status_id": "...", "if_match": {"<id>": 7} }
```

- Max 100 tasks per request.
- **Partial success is the contract**, and it is explicit: `207 Multi-Status` with
  a per-task result. Bulk operations across 100 tasks with individual permission
  and workflow rules will legitimately partially fail, and all-or-nothing would
  make the feature useless.
- Each task is its own transaction — one bad task does not roll back 99 good ones.
- Above 100, the client is directed to the async job endpoint, which returns a
  job id and reports progress.

The answer is `207` whatever the mix, all-success included: a client that must
parse per-task results anyway should not first branch on the status line.

```http
207 Multi-Status
{
  "results": [
    { "task_id": "...", "status": 200, "task": { … },
      "undo": { "to_status_id": "<where it was>", "if_match": 8 } },
    { "task_id": "...", "status": 409,
      "error": { "code": "TF-CNC-0001", "message": "…", "request_id": "…" } }
  ],
  "succeeded": 1,
  "failed": 1
}
```

- `status` is what the same operation would have returned on its own endpoint,
  and `error` is byte-for-byte the object that response carries — one renderer,
  not two.
- `undo` is present on every success, because a `207` where six of forty refused
  cannot be reversed by one inverse call. It is the status the task came *from*
  and the version it now holds: `POST` it back to
  `/tasks/{id}/transitions` to reverse that one task.
- **Malformed envelope → `400`; anything task-shaped → a row.** Unknown
  operation, no tasks, a repeated task, a version for a task not in `task_ids`,
  or over the limit are `400` — there is no row to report them on and the client
  could have known before sending. A missing `if_match` entry is that one task's
  `428`, not the batch's.
- `if_match` is a map rather than a header: forty tasks are at forty versions,
  and a single header could only be a wildcard.

## Rate limiting

Per actor, per workspace, token bucket ([21](21-API-LIMITS-AND-QUOTAS.md)).
Standard headers on every response:

```
RateLimit-Limit: 1000
RateLimit-Remaining: 847
RateLimit-Reset: 42
```

Returned on success too, so a client can slow down *before* being throttled.

## OpenAPI & compatibility

`GET /api/v1/openapi.json` serves the generated document. CI diffs it against the
committed snapshot; any breaking change (removed field, narrowed type, new
required request field) fails the build unless the PR also bumps the API version
([15](15-CI-AND-RELEASE-GATES.md)).

**Additive is safe, removal is not.** Adding a response field, an optional
request field, an enum value in a response, or an endpoint is minor. Clients must
tolerate unknown response fields — stated in the contract, and verified by a
client-compat test.
