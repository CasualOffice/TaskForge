-- 0004 — Workflow, projects, and project satellites.
-- See docs/23-WORKFLOW-AND-STATE-MACHINE.md, docs/03-DOMAIN-MODEL.md.
--
-- Workflow is created before project because a project requires one.

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
-- Exactly one initial status per workflow (docs/23).
CREATE UNIQUE INDEX workflow_initial_uq
    ON workflow_status (workflow_id) WHERE is_initial;

CREATE TABLE workflow_transition (
    id                  uuid PRIMARY KEY,
    workflow_id         uuid NOT NULL REFERENCES workflow(id) ON DELETE CASCADE,
    workspace_id        uuid NOT NULL,
    -- NULL means "from any status" — how "Cancel from anywhere" is expressed
    -- without one row per source.
    from_status_id      uuid REFERENCES workflow_status(id),
    to_status_id        uuid NOT NULL REFERENCES workflow_status(id),
    required_permission text REFERENCES permission(key),
    required_fields     text[] NOT NULL DEFAULT '{}',
    ignore_dependencies boolean NOT NULL DEFAULT false
);
-- NULLS NOT DISTINCT so a second "from any" edge to the same target collides
-- rather than silently duplicating.
CREATE UNIQUE INDEX workflow_transition_uq
    ON workflow_transition (workflow_id, from_status_id, to_status_id) NULLS NOT DISTINCT;

CREATE TABLE project (
    id            uuid PRIMARY KEY,
    workspace_id  uuid NOT NULL REFERENCES workspace(id) ON DELETE CASCADE,
    team_id       uuid REFERENCES team(id),
    -- Immutable after creation (ADR-007): task keys appear in commits, chat, and
    -- external tickets. Renaming a project does not rename its key.
    key           text NOT NULL CHECK (key ~ '^[A-Z][A-Z0-9]{1,9}$'),
    name          text NOT NULL,
    description   text,
    visibility    visibility NOT NULL DEFAULT 'TEAM',
    workflow_id   uuid NOT NULL REFERENCES workflow(id),
    -- Allocated in-transaction (ADR-008). A sequence would leak numbers on
    -- rollback, and users read gaps as lost data.
    task_seq      bigint NOT NULL DEFAULT 0,
    created_at    timestamptz NOT NULL DEFAULT now(),
    created_by    uuid NOT NULL REFERENCES user_account(id),
    updated_at    timestamptz NOT NULL DEFAULT now(),
    updated_by    uuid REFERENCES user_account(id),
    version       bigint NOT NULL DEFAULT 1,
    archived_at   timestamptz,
    deleted_at    timestamptz,
    UNIQUE (workspace_id, key)
);
CREATE INDEX project_ws_ix ON project (workspace_id, archived_at) WHERE deleted_at IS NULL;

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

CREATE TABLE milestone (
    id            uuid PRIMARY KEY,
    workspace_id  uuid NOT NULL REFERENCES workspace(id) ON DELETE CASCADE,
    project_id    uuid NOT NULL REFERENCES project(id) ON DELETE CASCADE,
    name          text NOT NULL,
    due_at        timestamptz,
    completed_at  timestamptz,
    UNIQUE (project_id, name)
);
