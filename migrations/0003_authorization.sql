-- 0003 — Authorization. See docs/04-RBAC-AND-AUTHORIZATION.md.
--
-- role_assignment is the ONLY source of authority in the system. No permission
-- is granted anywhere else — not by a boolean column, not by an is_admin flag,
-- and not by project membership. project_membership conveys belonging, never
-- capability.

CREATE TABLE permission (          -- seeded reference data, not user data
    key           text PRIMARY KEY,
    description   text NOT NULL,
    added_in      text NOT NULL     -- API version
);

CREATE TABLE role (
    id            uuid PRIMARY KEY,
    workspace_id  uuid NOT NULL REFERENCES workspace(id) ON DELETE CASCADE,
    name          text NOT NULL,
    -- Templates are cloneable starting points, not special-cased code. Nothing
    -- in the resolver knows a role is "built-in".
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
    scope_id       uuid NOT NULL,               -- = workspace_id at WORKSPACE scope
    constraints    jsonb NOT NULL DEFAULT '{}', -- closed set, validated on write
    granted_by     uuid NOT NULL REFERENCES user_account(id),
    granted_at     timestamptz NOT NULL DEFAULT now(),
    -- Makes granting idempotent, which matters because the UI retries.
    UNIQUE (workspace_id, principal_type, principal_id, role_id, scope_type, scope_id)
);

-- The resolver's hot path (docs/26 §Authorization tables).
CREATE INDEX role_assignment_lookup_ix
    ON role_assignment (workspace_id, principal_type, principal_id, scope_type, scope_id);
-- "who has access to this project"
CREATE INDEX role_assignment_scope_ix
    ON role_assignment (workspace_id, scope_type, scope_id);
CREATE INDEX role_permission_role_ix ON role_permission (role_id);
