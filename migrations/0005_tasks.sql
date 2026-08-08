-- 0005 — The universal work item and its satellites.
-- See docs/03-DOMAIN-MODEL.md, docs/26-SEARCH-INDEXING-AND-QUERY.md.

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
    -- Derived from status and written in the SAME statement, so it can never
    -- drift. This is the invariant that lets every report read state without a
    -- join (docs/23).
    state          task_state NOT NULL,
    reporter_id    uuid NOT NULL REFERENCES user_account(id),
    environment_id uuid REFERENCES project_environment(id),
    milestone_id   uuid REFERENCES milestone(id),
    parent_id      uuid REFERENCES task(id),
    start_at       timestamptz,
    due_at         timestamptz,
    position       text NOT NULL,                -- lexicographic rank (ADR-013)
    created_at     timestamptz NOT NULL DEFAULT now(),
    created_by     uuid NOT NULL REFERENCES user_account(id),
    updated_at     timestamptz NOT NULL DEFAULT now(),
    updated_by     uuid REFERENCES user_account(id),
    version        bigint NOT NULL DEFAULT 1,    -- optimistic concurrency (ADR-023)
    archived_at    timestamptz,
    deleted_at     timestamptz,
    UNIQUE (project_id, number)
);

-- The index inventory from docs/26 §task. Partial on deleted_at because
-- soft-deleted rows are a minority forever, and excluding them keeps every hot
-- index smaller.
CREATE INDEX task_board_ix      ON task (project_id, status_id, position)  WHERE deleted_at IS NULL;
CREATE INDEX task_list_ix       ON task (project_id, updated_at DESC, id DESC) WHERE deleted_at IS NULL;
CREATE INDEX task_mywork_ix     ON task (workspace_id, state, due_at)      WHERE deleted_at IS NULL;
CREATE INDEX task_reporter_ix   ON task (workspace_id, reporter_id)        WHERE deleted_at IS NULL;
CREATE INDEX task_parent_ix     ON task (parent_id)                        WHERE parent_id IS NOT NULL;
CREATE INDEX task_milestone_ix  ON task (milestone_id)                     WHERE milestone_id IS NOT NULL;
CREATE INDEX task_env_ix        ON task (project_id, environment_id)       WHERE environment_id IS NOT NULL;
CREATE INDEX task_due_ix        ON task (workspace_id, due_at)             WHERE due_at IS NOT NULL AND deleted_at IS NULL;
CREATE INDEX task_type_prio_ix  ON task (project_id, type, priority)       WHERE deleted_at IS NULL;
-- Cheap over a large append-mostly table; serves analytics and archival sweeps.
CREATE INDEX task_updated_brin  ON task USING brin (updated_at);

CREATE TABLE task_assignee (
    task_id       uuid NOT NULL REFERENCES task(id) ON DELETE CASCADE,
    user_id       uuid NOT NULL REFERENCES user_account(id),
    workspace_id  uuid NOT NULL,
    is_primary    boolean NOT NULL DEFAULT false,
    assigned_at   timestamptz NOT NULL DEFAULT now(),
    PRIMARY KEY (task_id, user_id)
);
CREATE INDEX task_assignee_user_ix ON task_assignee (user_id, workspace_id);
-- Multiple assignees, at most one primary (ADR-010).
CREATE UNIQUE INDEX task_primary_assignee_uq ON task_assignee (task_id) WHERE is_primary;

CREATE TABLE task_dependency (
    from_task_id  uuid NOT NULL REFERENCES task(id) ON DELETE CASCADE,
    to_task_id    uuid NOT NULL REFERENCES task(id) ON DELETE CASCADE,
    workspace_id  uuid NOT NULL,
    kind          dependency_kind NOT NULL DEFAULT 'BLOCKS',
    created_at    timestamptz NOT NULL DEFAULT now(),
    PRIMARY KEY (from_task_id, to_task_id),
    -- Trivial self-cycle only; deeper cycles are rejected in-transaction by a
    -- depth-limited reachability check under an advisory lock (docs/24).
    CHECK (from_task_id <> to_task_id)
);
CREATE INDEX task_dependency_rev_ix ON task_dependency (to_task_id);

CREATE TABLE tag (
    id            uuid PRIMARY KEY,
    workspace_id  uuid NOT NULL REFERENCES workspace(id) ON DELETE CASCADE,
    project_id    uuid REFERENCES project(id),   -- NULL = workspace-scoped
    name          citext NOT NULL,
    color         text,
    -- NULLS NOT DISTINCT (PG15+) is load-bearing: with default NULL semantics a
    -- plain unique constraint would permit unlimited duplicate workspace tags.
    UNIQUE NULLS NOT DISTINCT (workspace_id, project_id, name)
);

CREATE TABLE task_tag (
    task_id       uuid NOT NULL REFERENCES task(id) ON DELETE CASCADE,
    tag_id        uuid NOT NULL REFERENCES tag(id) ON DELETE CASCADE,
    workspace_id  uuid NOT NULL,
    PRIMARY KEY (task_id, tag_id)
);
-- The reverse direction a composite PK alone does not serve. Without it,
-- "show everything tagged security" scans (docs/26).
CREATE INDEX task_tag_rev_ix ON task_tag (tag_id, task_id);
