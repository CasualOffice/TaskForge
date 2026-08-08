# 32 — Tenancy & Isolation

The old drafts said "workspace ID is mandatory in every tenant query." That is a
statement of intent, not a mechanism — and intent does not survive the two
hundredth query written by the eleventh engineer. This doc specifies the
mechanisms that make it true.

**Two independent mechanisms must both fail to leak data across tenants.**

## Mechanism 1 — `WorkspaceScope` as a capability type

Tenancy is enforced by the type system before it is enforced by anything else.

```rust
/// Proof that the caller has been authenticated into a workspace.
/// Constructible only from an authenticated request.
#[derive(Clone, Copy)]
pub struct WorkspaceScope(WorkspaceId);

impl WorkspaceScope {
    /// The only public constructor. Lives in casual-task-model,
    /// callable only by the auth middleware.
    pub fn from_auth(ctx: &AuthContext) -> Self { WorkspaceScope(ctx.workspace_id) }

    pub fn id(&self) -> WorkspaceId { self.0 }
}
```

**Every repository method takes it:**

```rust
async fn find_task(&self, scope: WorkspaceScope, id: TaskId) -> Result<Option<Task>>;
async fn list_tasks(&self, scope: WorkspaceScope, filter: &CompiledFilter) -> Result<Page<Task>>;
```

and every implementation uses it:

```rust
sqlx::query_as!(TaskRow,
    "SELECT ... FROM task WHERE workspace_id = $1 AND id = $2",
    scope.id(), id)
```

The consequence: **you cannot write a repository call that forgets the tenant
filter, because you cannot obtain the argument without an authenticated context.**
This converts a review-discipline problem into a compile error.

There is no `WorkspaceScope::new(id)`, no `Default`, no `From<Uuid>`. Background
jobs receive a scope reconstructed from the job's recorded workspace — a job row
without a workspace cannot be enqueued, because the enqueue signature requires one.

## Mechanism 2 — row-level security as the backstop (ADR-020)

Every tenant table has RLS enabled:

```sql
ALTER TABLE task ENABLE ROW LEVEL SECURITY;
ALTER TABLE task FORCE ROW LEVEL SECURITY;

CREATE POLICY task_tenant_isolation ON task
    USING (workspace_id = current_setting('taskforge.workspace_id', true)::uuid);
```

The connection wrapper sets the variable from the scope on checkout, and resets it
on return:

```rust
sqlx::query("SELECT set_config('taskforge.workspace_id', $1, true)")
    .bind(scope.id().to_string()).execute(&mut *conn).await?;
```

`set_config(..., true)` is **transaction-local**, so a pooled connection cannot
carry one tenant's setting into another's transaction — the classic pooling bug
in RLS deployments.

**RLS is not the authorization engine.** It answers "is this row in my tenant,"
not "may this actor do this" ([04](04-RBAC-AND-AUTHORIZATION.md)). Trying to
express roles and constraints in policy functions produces logic that cannot be
unit-tested and cannot be explained to a user.

Migrations and the retention worker run as a separate role with `BYPASSRLS`, which
is never used by request-serving code.

## Isolation across every other surface

`workspace_id` is not only a database column. Each of these was an explicit
failure mode in some real system:

| Surface | Rule |
| --- | --- |
| **Cache keys** | Always prefixed `ws:{workspace_id}:…`. A cache key without it fails a lint. |
| **Object storage** | Keys are `{workspace_id}/{task_id}/{attachment_id}`; pre-signed URLs are minted only for a key matching the caller's scope ([28](28-ATTACHMENT-PIPELINE.md)). |
| **Search** | `task_search.workspace_id` is the first predicate of every query ([26](26-SEARCH-INDEXING-AND-QUERY.md)). |
| **Background jobs** | The job payload type requires a `WorkspaceId` field; it cannot be constructed without one. |
| **SSE streams** | A subscription is bound to one workspace and revalidated on `authz_epoch` change ([05](05-API-SPEC.md)). |
| **Plugin tokens** | Issued per installation, per workspace. A plugin installed in two workspaces gets two tokens and cannot correlate them. |
| **Rate limits** | Bucketed per `(workspace, actor)`, so a noisy tenant cannot exhaust another's quota. |
| **Metrics & traces** | `workspace_id` is a low-cardinality-safe *hashed* label; raw IDs go to logs, not to metric labels. |
| **Error messages** | Never echo an ID from another tenant. A 404 for a foreign resource is indistinguishable from a 404 for an absent one. |

## The `user_account` exception

`user_account` is the only table without `workspace_id` — a person legitimately
exists across workspaces. This is a real cross-tenant surface and is handled
explicitly:

- Reads always go through `workspace_membership`; there is no repository method
  that returns a user without a scope.
- **User search never leaks membership.** Typeahead in workspace A returns only
  members of A. Searching for a colleague's email in a workspace they do not
  belong to returns nothing — not "exists but not a member."
- Invite-by-email must not reveal whether the address has an account. The response
  is identical either way; the difference is only in the email the recipient gets.

That last point is a genuine account-enumeration channel, and it is the one most
commonly shipped by accident.

## Deletion and residency

**Workspace deletion** is staged: soft-delete → 30-day grace (restorable, hidden,
billing stopped) → hard delete. Hard delete removes every tenant row via
`ON DELETE CASCADE` from `workspace`, drops the object-store prefix, purges cache
keys by prefix, cancels queued jobs, and revokes plugin tokens. It emits a final
audit record to a retained store *outside* the deleted tenant — otherwise the
evidence of deletion is deleted by the deletion.

**User deletion** anonymizes in place (ADR-026): the account becomes a tombstone,
email nulled, PII scrubbed; authored rows keep their foreign keys. Erasing
history to remove one person would destroy the audit trail for everyone else
([07](07-QUALITY-SECURITY-AND-COMPATIBILITY.md)).

**Data residency is not designed and must not be promised.** Multi-region
placement affects the schema (region as a routing key), the object store, and
backup topology. Committing to it for a customer before the ADR exists would be a
schema change under deadline — the worst way to make it. Flagged in
[08](08-ADR-REGISTER.md) as pending.

## Noisy-neighbour containment

Isolation is also about one tenant not degrading another:

- Per-workspace rate limits and quotas ([21](21-API-LIMITS-AND-QUOTAS.md)).
- Statement timeouts, so one pathological query cannot hold a connection.
- Bounded worker concurrency **per workspace**, so a bulk import in one tenant
  does not starve every other tenant's outbox dispatch.
- Attachment storage quotas per workspace.

## Acceptance gates

- **Cross-tenant property test** — the central one: seed two workspaces with
  identical-shaped data, then exercise **every** endpoint as a member of A
  requesting B's resource IDs. Every response is `404`, never `403`, never data.
  Generated from the route table so a new endpoint is automatically covered — a
  new route cannot be added without being tested for this.
- **RLS backstop test** — deliberately issue a repository query with the tenant
  predicate removed; assert zero rows returned (proving RLS catches what the type
  system would have prevented).
- **Pool-leak test** — interleave transactions from two workspaces on one pooled
  connection; assert no setting bleed.
- **Cache-key lint** — a build-time check that every cache key constructor takes a
  `WorkspaceScope`.
- **Enumeration test** — invite, login, and user-search responses are
  byte-identical for existing and non-existing accounts.
- **Deletion test** — after hard delete, no row, object, cache entry, or queued
  job referencing the workspace remains; the external audit record does.
