# 22 — Database Schema

The physical schema. Meaning lives in [03](03-DOMAIN-MODEL.md); indexes and their
rationale live in [26](26-SEARCH-INDEXING-AND-QUERY.md). This doc is the DDL
contract that migrations must produce.

## Conventions

- **PostgreSQL 16+.** Extensions: `pg_trgm` (fuzzy search), `btree_gin`.
  `uuidv7()` is generated application-side (Rust `uuid` crate) so IDs are
  assigned before insert and usable in the same transaction.
- **Every tenant table** carries `workspace_id uuid NOT NULL`, plus
  `created_at`, `created_by`, `updated_at`, `updated_by`, and `version` where
  mutable.
- **`timestamptz` everywhere.** No naive timestamps, ever.
- **Soft delete** via `deleted_at timestamptz`; every hot index is partial on
  `WHERE deleted_at IS NULL`.
- **Enums are Postgres enum types** where the set is closed forever (`task_state`),
  and `text` + `CHECK` where it may grow (membership types). Adding an enum value
  is cheap; removing one is not — that asymmetry decides which is used.
- **Naming:** singular table names, `snake_case`, `_id` foreign keys,
  `_at` timestamps. Indexes `{table}_{purpose}_{ix|uq}`.
- **Migrations** are versioned SQL run by `sqlx migrate`, forward-only. Every
  migration is tested against a seeded prior version in CI.

## Types

```sql
CREATE TYPE task_state    AS ENUM ('BACKLOG','PLANNED','ACTIVE','COMPLETED','CANCELED');
CREATE TYPE task_type     AS ENUM ('TASK','BUG','FEATURE','INCIDENT','REQUEST');
CREATE TYPE task_priority AS ENUM ('NONE','LOW','MEDIUM','HIGH','URGENT');
CREATE TYPE visibility    AS ENUM ('PRIVATE','TEAM','WORKSPACE');
CREATE TYPE principal_type AS ENUM ('USER','TEAM','SERVICE_ACCOUNT');
CREATE TYPE scope_type    AS ENUM ('WORKSPACE','TEAM','PROJECT','ENVIRONMENT');
CREATE TYPE dependency_kind AS ENUM ('BLOCKS');
```

`task_priority` is an enum rather than an integer so `ORDER BY priority DESC`
sorts semantically without a lookup, and so adding a level later is a type
change reviewed by ADR rather than a magic number.

## Tenancy & identity

```sql
CREATE TABLE workspace (
    id            uuid PRIMARY KEY,
    name          text NOT NULL,
    slug          text NOT NULL UNIQUE,
    authz_epoch   bigint NOT NULL DEFAULT 1,   -- bumped on any grant/membership change
    version       bigint NOT NULL DEFAULT 1,   -- optimistic concurrency (migration 0019)
    settings      jsonb NOT NULL DEFAULT '{}',
    created_at    timestamptz NOT NULL DEFAULT now(),
    deleted_at    timestamptz
);

CREATE TABLE user_account (
    id            uuid PRIMARY KEY,
    email         citext UNIQUE,               -- NULL once anonymized
    display_name  text NOT NULL,
    avatar_url    text,
    is_tombstone  boolean NOT NULL DEFAULT false,
    created_at    timestamptz NOT NULL DEFAULT now(),
    updated_at    timestamptz NOT NULL DEFAULT now()
);

CREATE TABLE workspace_membership (
    workspace_id  uuid NOT NULL REFERENCES workspace(id) ON DELETE CASCADE,
    user_id       uuid NOT NULL REFERENCES user_account(id),
    member_type   text NOT NULL CHECK (member_type IN ('MEMBER','GUEST')),
    joined_at     timestamptz NOT NULL DEFAULT now(),
    PRIMARY KEY (workspace_id, user_id)
);

CREATE TABLE team (
    id            uuid PRIMARY KEY,
    workspace_id  uuid NOT NULL REFERENCES workspace(id) ON DELETE CASCADE,
    name          text NOT NULL,
    version       bigint NOT NULL DEFAULT 1,   -- optimistic concurrency (migration 0019)
    created_at    timestamptz NOT NULL DEFAULT now(),
    deleted_at    timestamptz,
    UNIQUE (workspace_id, name)
);

CREATE TABLE team_membership (
    team_id       uuid NOT NULL REFERENCES team(id) ON DELETE CASCADE,
    user_id       uuid NOT NULL REFERENCES user_account(id),
    PRIMARY KEY (team_id, user_id)
);
CREATE INDEX team_membership_user_ix ON team_membership (user_id);
```

`user_account` is the **only** table without `workspace_id` — a person exists
across workspaces. Every path that reads it does so through a membership.

## Authorization

```sql
CREATE TABLE permission (                       -- seeded reference data, not user data
    key           text PRIMARY KEY,             -- 'task.close'
    description   text NOT NULL,
    added_in      text NOT NULL                 -- API version
);

CREATE TABLE role (
    id            uuid PRIMARY KEY,
    workspace_id  uuid NOT NULL REFERENCES workspace(id) ON DELETE CASCADE,
    name          text NOT NULL,
    is_template   boolean NOT NULL DEFAULT false,
    created_at    timestamptz NOT NULL DEFAULT now(),
    updated_at    timestamptz NOT NULL DEFAULT now(),
    version       bigint NOT NULL DEFAULT 1,
    UNIQUE (workspace_id, name)
);

CREATE TABLE role_permission (
    role_id       uuid NOT NULL REFERENCES role(id) ON DELETE CASCADE,
    permission    text NOT NULL REFERENCES permission(key),
    PRIMARY KEY (role_id, permission)
);

CREATE TABLE role_assignment (
    id             uuid PRIMARY KEY,
    workspace_id   uuid NOT NULL REFERENCES workspace(id) ON DELETE CASCADE,
    principal_type principal_type NOT NULL,
    principal_id   uuid NOT NULL,
    role_id        uuid NOT NULL REFERENCES role(id) ON DELETE CASCADE,
    scope_type     scope_type NOT NULL,
    scope_id       uuid NOT NULL,               -- = workspace_id when scope is WORKSPACE
    constraints    jsonb NOT NULL DEFAULT '{}', -- closed constraint set, validated on write
    granted_by     uuid NOT NULL REFERENCES user_account(id),
    granted_at     timestamptz NOT NULL DEFAULT now(),
    UNIQUE (workspace_id, principal_type, principal_id, role_id, scope_type, scope_id)
);

CREATE INDEX role_assignment_lookup_ix
    ON role_assignment (workspace_id, principal_type, principal_id, scope_type, scope_id);
CREATE INDEX role_assignment_scope_ix
    ON role_assignment (workspace_id, scope_type, scope_id);
```

`role_assignment` is the **only** source of authority in the system. No permission
is granted anywhere else — not by a boolean column, not by an `is_admin` flag, not
by project membership. `project_membership` conveys *belonging*, never capability.

The `UNIQUE` constraint makes granting idempotent, which matters because the UI
retries.

## Projects & workflow

```sql
CREATE TABLE project (
    id            uuid PRIMARY KEY,
    workspace_id  uuid NOT NULL REFERENCES workspace(id) ON DELETE CASCADE,
    team_id       uuid REFERENCES team(id),
    key           text NOT NULL CHECK (key ~ '^[A-Z][A-Z0-9]{1,9}$'),  -- immutable, ADR-007
    name          text NOT NULL,
    description   text,
    visibility    visibility NOT NULL DEFAULT 'TEAM',
    workflow_id   uuid NOT NULL,                -- FK added after workflow table
    task_seq      bigint NOT NULL DEFAULT 0,    -- in-transaction allocation, ADR-008
    created_at    timestamptz NOT NULL DEFAULT now(),
    created_by    uuid NOT NULL REFERENCES user_account(id),
    updated_at    timestamptz NOT NULL DEFAULT now(),
    updated_by    uuid REFERENCES user_account(id),
    version       bigint NOT NULL DEFAULT 1,
    archived_at   timestamptz,
    deleted_at    timestamptz,
    UNIQUE (workspace_id, key)
);

CREATE TABLE project_membership (
    project_id    uuid NOT NULL REFERENCES project(id) ON DELETE CASCADE,
    user_id       uuid NOT NULL REFERENCES user_account(id),
    workspace_id  uuid NOT NULL,
    added_at      timestamptz NOT NULL DEFAULT now(),
    PRIMARY KEY (project_id, user_id)
);
CREATE INDEX project_membership_user_ix ON project_membership (user_id, workspace_id);

CREATE TABLE project_environment (
    id            uuid PRIMARY KEY,
    project_id    uuid NOT NULL REFERENCES project(id) ON DELETE CASCADE,
    workspace_id  uuid NOT NULL,
    name          text NOT NULL,
    position      integer NOT NULL,
    UNIQUE (project_id, name)
);

CREATE TABLE workflow (
    id            uuid PRIMARY KEY,
    workspace_id  uuid NOT NULL REFERENCES workspace(id) ON DELETE CASCADE,
    name          text NOT NULL,
    is_default    boolean NOT NULL DEFAULT false,
    version       bigint NOT NULL DEFAULT 1
);

CREATE TABLE workflow_status (
    id            uuid PRIMARY KEY,
    workflow_id   uuid NOT NULL REFERENCES workflow(id) ON DELETE CASCADE,
    workspace_id  uuid NOT NULL,
    name          text NOT NULL,
    state         task_state NOT NULL,          -- the permanent contract
    position      integer NOT NULL,
    is_initial    boolean NOT NULL DEFAULT false,
    UNIQUE (workflow_id, name)
);

CREATE TABLE workflow_transition (
    id                  uuid PRIMARY KEY,
    workflow_id         uuid NOT NULL REFERENCES workflow(id) ON DELETE CASCADE,
    workspace_id        uuid NOT NULL,
    from_status_id      uuid REFERENCES workflow_status(id),  -- NULL = from any
    to_status_id        uuid NOT NULL REFERENCES workflow_status(id),
    required_permission text REFERENCES permission(key),
    required_fields     text[] NOT NULL DEFAULT '{}',
    ignore_dependencies boolean NOT NULL DEFAULT false,
    UNIQUE (workflow_id, from_status_id, to_status_id)
);

ALTER TABLE project ADD CONSTRAINT project_workflow_fk
    FOREIGN KEY (workflow_id) REFERENCES workflow(id);

-- exactly one initial status per workflow
CREATE UNIQUE INDEX workflow_initial_uq
    ON workflow_status (workflow_id) WHERE is_initial;

-- exactly one DEFAULT workflow per workspace (migration 0019, C-006)
CREATE UNIQUE INDEX workflow_default_uq
    ON workflow (workspace_id) WHERE is_default;

-- the project list's keyset order (migration 0019, C-006)
CREATE INDEX project_list_ix
    ON project (workspace_id, created_at DESC, id DESC) WHERE deleted_at IS NULL;
```

`workflow_default_uq` is what makes lazy provisioning safe. `project.workflow_id`
is `NOT NULL` and nothing creates a workflow, so the first project create in a
workspace materializes the default one ([23](23-WORKFLOW-AND-STATE-MACHINE.md)
§The default workflow) inside its own transaction. Two concurrent first creates
would each insert one, and the workspace would end up with two workflows both
claiming to be the default — with no error anywhere. A check-then-insert cannot
prevent that; a unique index can.

## Task

```sql
CREATE TABLE task (
    id             uuid PRIMARY KEY,
    workspace_id   uuid NOT NULL REFERENCES workspace(id) ON DELETE CASCADE,
    project_id     uuid NOT NULL REFERENCES project(id),
    number         bigint NOT NULL,
    title          text NOT NULL CHECK (length(title) BETWEEN 1 AND 512),
    description    text CHECK (description IS NULL OR length(description) <= 65536),
    type           task_type NOT NULL DEFAULT 'TASK',
    priority       task_priority NOT NULL DEFAULT 'NONE',
    status_id      uuid NOT NULL REFERENCES workflow_status(id),
    state          task_state NOT NULL,          -- derived from status, same statement
    reporter_id    uuid NOT NULL REFERENCES user_account(id),
    environment_id uuid REFERENCES project_environment(id),
    milestone_id   uuid REFERENCES milestone(id),
    parent_id      uuid REFERENCES task(id),
    start_at       timestamptz,
    due_at         timestamptz,
    position       text NOT NULL,                -- lexicographic rank, ADR-013
    created_at     timestamptz NOT NULL DEFAULT now(),
    created_by     uuid NOT NULL REFERENCES user_account(id),
    updated_at     timestamptz NOT NULL DEFAULT now(),
    updated_by     uuid REFERENCES user_account(id),
    version        bigint NOT NULL DEFAULT 1,    -- optimistic concurrency
    archived_at    timestamptz,
    deleted_at     timestamptz,
    UNIQUE (project_id, number)
);

CREATE TABLE task_assignee (
    task_id       uuid NOT NULL REFERENCES task(id) ON DELETE CASCADE,
    user_id       uuid NOT NULL REFERENCES user_account(id),
    workspace_id  uuid NOT NULL,
    is_primary    boolean NOT NULL DEFAULT false,
    assigned_at   timestamptz NOT NULL DEFAULT now(),
    PRIMARY KEY (task_id, user_id)
);
CREATE INDEX task_assignee_user_ix ON task_assignee (user_id, workspace_id);
-- at most one primary assignee per task
CREATE UNIQUE INDEX task_primary_assignee_uq
    ON task_assignee (task_id) WHERE is_primary;

CREATE TABLE task_dependency (
    from_task_id  uuid NOT NULL REFERENCES task(id) ON DELETE CASCADE,
    to_task_id    uuid NOT NULL REFERENCES task(id) ON DELETE CASCADE,
    workspace_id  uuid NOT NULL,
    kind          dependency_kind NOT NULL DEFAULT 'BLOCKS',
    created_at    timestamptz NOT NULL DEFAULT now(),
    PRIMARY KEY (from_task_id, to_task_id),
    CHECK (from_task_id <> to_task_id)          -- trivial self-cycle; deeper cycles checked in-txn
);
CREATE INDEX task_dependency_rev_ix ON task_dependency (to_task_id);

CREATE TABLE tag (
    id            uuid PRIMARY KEY,
    workspace_id  uuid NOT NULL REFERENCES workspace(id) ON DELETE CASCADE,
    project_id    uuid REFERENCES project(id),   -- NULL = workspace-scoped
    name          citext NOT NULL,
    color         text,
    UNIQUE NULLS NOT DISTINCT (workspace_id, project_id, name)
);

CREATE TABLE task_tag (
    task_id       uuid NOT NULL REFERENCES task(id) ON DELETE CASCADE,
    tag_id        uuid NOT NULL REFERENCES tag(id) ON DELETE CASCADE,
    workspace_id  uuid NOT NULL,
    PRIMARY KEY (task_id, tag_id)
);
CREATE INDEX task_tag_rev_ix ON task_tag (tag_id, task_id);   -- the reverse direction

CREATE TABLE milestone (
    id            uuid PRIMARY KEY,
    workspace_id  uuid NOT NULL REFERENCES workspace(id) ON DELETE CASCADE,
    project_id    uuid NOT NULL REFERENCES project(id) ON DELETE CASCADE,
    name          text NOT NULL,
    due_at        timestamptz,
    completed_at  timestamptz,
    UNIQUE (project_id, name)
);
```

`UNIQUE NULLS NOT DISTINCT` (PG 15+) makes workspace-scoped tags (`project_id IS
NULL`) actually unique — with default NULL semantics, a plain unique constraint
would permit unlimited duplicates.

## Comments & attachments

```sql
CREATE TABLE comment (
    id                uuid PRIMARY KEY,
    workspace_id      uuid NOT NULL REFERENCES workspace(id) ON DELETE CASCADE,
    task_id           uuid NOT NULL REFERENCES task(id) ON DELETE CASCADE,
    parent_comment_id uuid REFERENCES comment(id),   -- one level of threading
    author_id         uuid NOT NULL REFERENCES user_account(id),
    body              text NOT NULL CHECK (length(body) <= 65536),
    mentions          uuid[] NOT NULL DEFAULT '{}',  -- resolved at write time
    created_at        timestamptz NOT NULL DEFAULT now(),
    edited_at         timestamptz,
    deleted_at        timestamptz,
    version           bigint NOT NULL DEFAULT 1
);
CREATE INDEX comment_task_ix ON comment (task_id, created_at) WHERE deleted_at IS NULL;

CREATE TABLE attachment (
    id            uuid PRIMARY KEY,
    workspace_id  uuid NOT NULL REFERENCES workspace(id) ON DELETE CASCADE,
    task_id       uuid NOT NULL REFERENCES task(id) ON DELETE CASCADE,
    object_key    text NOT NULL UNIQUE,           -- workspace-prefixed
    filename      text NOT NULL,
    content_type  text NOT NULL,
    byte_size     bigint NOT NULL,
    checksum      text NOT NULL,                  -- sha256
    scan_status   text NOT NULL DEFAULT 'PENDING'
                  CHECK (scan_status IN ('PENDING','CLEAN','INFECTED','FAILED')),
    committed_at  timestamptz,                    -- NULL ⇒ invisible everywhere
    uploaded_by   uuid NOT NULL REFERENCES user_account(id),
    created_at    timestamptz NOT NULL DEFAULT now(),
    deleted_at    timestamptz
);
CREATE INDEX attachment_task_ix ON attachment (task_id)
    WHERE committed_at IS NOT NULL AND deleted_at IS NULL;
```

The attachment index is partial on `committed_at IS NOT NULL` — uncommitted rows
are not merely filtered out at read time, they are **not in the index that reads
use**, so an abandoned upload cannot leak through a missing predicate.

## Events, activity, audit

```sql
CREATE TABLE outbox_event (
    id             uuid PRIMARY KEY,
    workspace_id   uuid NOT NULL,
    event_type     text NOT NULL,
    aggregate_type text NOT NULL,
    aggregate_id   uuid NOT NULL,
    payload        jsonb NOT NULL,
    schema_version integer NOT NULL,
    created_at     timestamptz NOT NULL DEFAULT now(),
    dispatched_at  timestamptz,
    attempts       integer NOT NULL DEFAULT 0,
    last_error     text
);
-- Delivery state is per (event, consumer) — see migration 0013 and docs/25
-- §Per-consumer delivery state. outbox_event keeps only the immutable fact;
-- its dispatched_at/attempts/last_error columns were dropped there.
CREATE INDEX outbox_delivery_pending_ix ON outbox_delivery (consumer, next_attempt_at, created_at)
    WHERE dispatched_at IS NULL AND dead_lettered_at IS NULL;

CREATE TABLE activity_event (
    id             uuid PRIMARY KEY,
    workspace_id   uuid NOT NULL,
    project_id     uuid,
    aggregate_type text NOT NULL,
    aggregate_id   uuid NOT NULL,
    event_type     text NOT NULL,
    actor_id       uuid REFERENCES user_account(id),   -- NULL = system
    changes        jsonb NOT NULL DEFAULT '{}',        -- {field: {from, to}}
    occurred_at    timestamptz NOT NULL DEFAULT now()
) PARTITION BY RANGE (occurred_at);

CREATE TABLE audit_event (
    id             uuid PRIMARY KEY,
    workspace_id   uuid NOT NULL,
    event_type     text NOT NULL,
    actor_id       uuid,
    actor_type     text NOT NULL,                      -- USER | SERVICE_ACCOUNT | PLUGIN | SYSTEM
    target_type    text,
    target_id      uuid,
    changes        jsonb NOT NULL DEFAULT '{}',
    request_id     uuid,
    correlation_id uuid,
    ip_address     inet,
    user_agent     text,
    occurred_at    timestamptz NOT NULL DEFAULT now()
) PARTITION BY RANGE (occurred_at);
```

Both event tables are **monthly range-partitioned**, so retention is
`DROP TABLE activity_event_2026_08` rather than a `DELETE` of tens of millions of
rows that bloats and vacuums for hours.

**Append-only is enforced by grant, not by convention** — the application role
holds `INSERT` and `SELECT` on these tables and nothing else:

```sql
REVOKE UPDATE, DELETE ON activity_event, audit_event FROM taskforge_app;
```

## Views, notifications, automation, plugins

```sql
CREATE TABLE saved_view (
    id            uuid PRIMARY KEY,
    workspace_id  uuid NOT NULL REFERENCES workspace(id) ON DELETE CASCADE,
    project_id    uuid REFERENCES project(id),
    owner_id      uuid NOT NULL REFERENCES user_account(id),
    name          text NOT NULL,
    filter        jsonb NOT NULL,                -- typed grammar, doc 27
    sort          jsonb NOT NULL DEFAULT '[]',
    layout        text NOT NULL DEFAULT 'LIST',
    shared        boolean NOT NULL DEFAULT false,
    created_at    timestamptz NOT NULL DEFAULT now(),
    version       bigint NOT NULL DEFAULT 1
);
CREATE INDEX saved_view_owner_ix ON saved_view (workspace_id, owner_id);

CREATE TABLE notification (
    id            uuid PRIMARY KEY,
    workspace_id  uuid NOT NULL REFERENCES workspace(id) ON DELETE CASCADE,
    user_id       uuid NOT NULL REFERENCES user_account(id),
    event_type    text NOT NULL,
    aggregate_id  uuid,
    payload       jsonb NOT NULL,
    created_at    timestamptz NOT NULL DEFAULT now(),
    read_at       timestamptz
);
CREATE INDEX notification_unread_ix
    ON notification (user_id, created_at DESC) WHERE read_at IS NULL;

CREATE TABLE automation_rule (
    id            uuid PRIMARY KEY,
    workspace_id  uuid NOT NULL REFERENCES workspace(id) ON DELETE CASCADE,
    project_id    uuid REFERENCES project(id),
    name          text NOT NULL,
    trigger       jsonb NOT NULL,
    conditions    jsonb NOT NULL DEFAULT '[]',
    actions       jsonb NOT NULL,
    enabled       boolean NOT NULL DEFAULT true,
    run_as        uuid NOT NULL REFERENCES user_account(id),   -- permission ceiling
    version       bigint NOT NULL DEFAULT 1
);

CREATE TABLE plugin_installation (
    id                uuid PRIMARY KEY,
    workspace_id      uuid NOT NULL REFERENCES workspace(id) ON DELETE CASCADE,
    plugin_id         text NOT NULL,             -- reverse-DNS
    version           text NOT NULL,
    manifest_hash     text NOT NULL,             -- what was consented to
    granted_scopes    text[] NOT NULL,
    config            jsonb NOT NULL DEFAULT '{}',
    secret_ref        text NOT NULL,             -- KMS/vault handle, never the secret
    installed_by      uuid NOT NULL REFERENCES user_account(id),
    installed_at      timestamptz NOT NULL DEFAULT now(),
    enabled           boolean NOT NULL DEFAULT true,
    uninstalled_at    timestamptz,               -- grace period starts here
    UNIQUE (workspace_id, plugin_id)
);

CREATE TABLE service_account (
    id            uuid PRIMARY KEY,
    workspace_id  uuid NOT NULL REFERENCES workspace(id) ON DELETE CASCADE,
    name          text NOT NULL,
    created_by    uuid NOT NULL REFERENCES user_account(id),
    disabled_at   timestamptz
);

CREATE TABLE api_token (
    id            uuid PRIMARY KEY,
    workspace_id  uuid NOT NULL REFERENCES workspace(id) ON DELETE CASCADE,
    principal_type principal_type NOT NULL,
    principal_id  uuid NOT NULL,
    token_hash    text NOT NULL UNIQUE,          -- argon2id; the token itself is never stored
    name          text NOT NULL,
    last_used_at  timestamptz,
    expires_at    timestamptz,
    revoked_at    timestamptz
);

CREATE TABLE idempotency_key (
    workspace_id  uuid NOT NULL,
    actor_id      uuid NOT NULL,
    key           text NOT NULL,
    request_hash  text NOT NULL,                 -- detects key reuse with a different body
    response      jsonb,
    status_code   integer,
    created_at    timestamptz NOT NULL DEFAULT now(),
    PRIMARY KEY (workspace_id, actor_id, key)
);
```

`automation_rule.run_as` is the fix for the classic automation privilege bug: a
rule executes with a **named principal's** permissions, not with the permissions
of whoever happened to trigger it, and not as an implicit superuser. If the
`run_as` user loses access, the rule fails visibly rather than silently
escalating ([36](36-AUTOMATION-RULES-DESIGN.md)).

`api_token` stores only a hash. The token is shown once, at creation, and is
unrecoverable — matching the industry norm and removing the "database dump equals
credential theft" path.

## Search projection

Defined in [26](26-SEARCH-INDEXING-AND-QUERY.md); repeated here for schema
completeness:

```sql
CREATE TABLE task_search (
    task_id       uuid PRIMARY KEY REFERENCES task(id) ON DELETE CASCADE,
    workspace_id  uuid NOT NULL,
    project_id    uuid NOT NULL,
    document      tsvector NOT NULL,
    title_trgm    text NOT NULL,
    updated_at    timestamptz NOT NULL
);
CREATE INDEX task_search_gin   ON task_search USING gin (document);
CREATE INDEX task_search_trgm  ON task_search USING gin (title_trgm gin_trgm_ops);
CREATE INDEX task_search_scope_ix ON task_search (workspace_id, project_id);
```

## Row-level security — the backstop

RLS is enabled on every tenant table as **defense in depth behind** the
type-level `WorkspaceScope` ([32](32-TENANCY-AND-ISOLATION.md)), not instead of it:

```sql
ALTER TABLE task ENABLE ROW LEVEL SECURITY;
CREATE POLICY task_tenant_isolation ON task
    USING (workspace_id = current_setting('taskforge.workspace_id')::uuid);
```

The session variable is set by the connection wrapper from the authenticated
scope. If application code ever forgets a `WHERE workspace_id`, RLS returns zero
rows instead of another tenant's data. Two independent mechanisms must both fail
to cause a cross-tenant leak.

## JSONB policy

JSONB is used in exactly five places, each with a validated schema and a written
justification:

| Column | Why JSONB is correct |
| --- | --- |
| `workspace.settings` | Sparse, admin-only, never queried by field |
| `saved_view.filter` / `.sort` | A typed grammar with recursive shape ([27](27-FILTER-AND-SAVED-VIEW-DSL.md)); validated on write |
| `automation_rule.*` | Same — a rule tree, never queried by field |
| `*_event.changes` / `payload` | Heterogeneous by definition; read whole |
| `plugin_installation.config` | Schema is the plugin's, validated against its manifest |

**Not** used for core task fields. Custom field *values* get a typed
`task_custom_field_value` table added in Phase 3 with the plugin contract, not a
JSONB blob on `task` — because they must be filterable and indexable
([26](26-SEARCH-INDEXING-AND-QUERY.md)).

## Migration rules

1. **Forward-only.** No down migrations; a bad migration is fixed by a new one.
2. **Expand → migrate → contract**, always, so a rollback of application code
   still runs against the new schema:
   add nullable column → backfill → start writing → start reading → drop old.
3. **No blocking DDL in a release path.** Indexes are built `CONCURRENTLY`;
   `NOT NULL` is added via a `CHECK ... NOT VALID` then `VALIDATE`.
4. **Every migration is tested against seeded production-shaped data** in CI, with
   a timing budget — a migration that would lock `task` for more than 1 s fails
   the build.
5. **Enum values are added, never removed**, within a major version.

## ADRs triggered

- **ADR-020** — PostgreSQL 16 with `pg_trgm`; RLS as a tenancy backstop.
- **ADR-021** — Monthly range partitioning for activity/audit; retention by
  partition drop.
- **ADR-022** — Confined JSONB policy; typed tables for anything filterable.
