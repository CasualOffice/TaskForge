-- 0008 — Saved views, notifications, automation, plugins, tokens, idempotency.
-- See docs/27, docs/29, docs/36, docs/34, docs/40, docs/24.

CREATE TABLE saved_view (
    id            uuid PRIMARY KEY,
    workspace_id  uuid NOT NULL REFERENCES workspace(id) ON DELETE CASCADE,
    project_id    uuid REFERENCES project(id),
    owner_id      uuid NOT NULL REFERENCES user_account(id),
    name          text NOT NULL,
    filter        jsonb NOT NULL,                -- typed grammar, docs/27
    sort          jsonb NOT NULL DEFAULT '[]',
    layout        text NOT NULL DEFAULT 'LIST' CHECK (layout IN ('LIST','BOARD','TABLE')),
    -- A shared view is visible to members but EXECUTES WITH THE VIEWER'S
    -- permissions. It is never a permission-bypass channel (docs/27).
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
    reason        text NOT NULL,     -- highest-ranked reason only (docs/29)
    aggregate_id  uuid,
    payload       jsonb NOT NULL,
    created_at    timestamptz NOT NULL DEFAULT now(),
    read_at       timestamptz
);
-- Serves the unread badge as an index-only count, not a scan.
CREATE INDEX notification_unread_ix ON notification (user_id, created_at DESC) WHERE read_at IS NULL;

CREATE TABLE automation_rule (
    id            uuid PRIMARY KEY,
    workspace_id  uuid NOT NULL REFERENCES workspace(id) ON DELETE CASCADE,
    project_id    uuid REFERENCES project(id),
    name          text NOT NULL,
    trigger       jsonb NOT NULL,
    conditions    jsonb NOT NULL DEFAULT '[]',   -- same AST as the filter grammar
    actions       jsonb NOT NULL,
    enabled       boolean NOT NULL DEFAULT true,
    -- Every rule executes as a NAMED principal, not as the triggering user and
    -- not as a superuser. Rule authoring is otherwise a privilege-escalation
    -- primitive (docs/36).
    run_as        uuid NOT NULL REFERENCES user_account(id),
    version       bigint NOT NULL DEFAULT 1
);
CREATE INDEX automation_rule_trigger_ix ON automation_rule (workspace_id, enabled);

CREATE TABLE plugin_installation (
    id                uuid PRIMARY KEY,
    workspace_id      uuid NOT NULL REFERENCES workspace(id) ON DELETE CASCADE,
    plugin_id         text NOT NULL,             -- reverse-DNS, immutable
    version           text NOT NULL,
    manifest_hash     text NOT NULL,             -- exactly what was consented to
    granted_scopes    text[] NOT NULL,
    config            jsonb NOT NULL DEFAULT '{}',
    secret_ref        text NOT NULL,             -- KMS/vault handle, never the secret
    installed_by      uuid NOT NULL REFERENCES user_account(id),
    installed_at      timestamptz NOT NULL DEFAULT now(),
    enabled           boolean NOT NULL DEFAULT true,
    uninstalled_at    timestamptz,               -- 30-day grace starts here
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
    id             uuid PRIMARY KEY,
    workspace_id   uuid NOT NULL REFERENCES workspace(id) ON DELETE CASCADE,
    principal_type principal_type NOT NULL,
    principal_id   uuid NOT NULL,
    -- argon2id hash. The plaintext is shown once and is unrecoverable, so a
    -- database dump is not a credential dump (docs/40).
    token_hash     text NOT NULL UNIQUE,
    name           text NOT NULL,
    last_used_at   timestamptz,
    expires_at     timestamptz,
    revoked_at     timestamptz
);

CREATE TABLE idempotency_key (
    workspace_id  uuid NOT NULL,
    actor_id      uuid NOT NULL,
    key           text NOT NULL,
    -- Catches the client bug of generating a key once and reusing it for a
    -- different task (docs/24).
    request_hash  text NOT NULL,
    response      jsonb,
    status_code   integer,
    created_at    timestamptz NOT NULL DEFAULT now(),
    PRIMARY KEY (workspace_id, actor_id, key)
);
CREATE INDEX idempotency_sweep_ix ON idempotency_key (created_at);
