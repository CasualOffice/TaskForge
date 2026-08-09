-- 0031 — Custody, environment promotion, releases and verification.
-- See docs/45-DEVELOPMENT-LIFECYCLE-AND-CUSTODY.md.
--
-- The product could describe a task's state and not its journey. Five stages of
-- the actual development lifecycle had nowhere to be recorded:
--
--   triage      — which TEAM owns this, and every time that answer changed
--   resolve     — which environment the fix was pushed to, with proof
--   verify      — tested on that environment: passed, or failed with evidence
--   promote     — qa → staging → production, usually many tasks at once
--   release     — what went out together
--
-- Two clocks, not one. `task.status_id` is what state the work is in;
-- `task.environment_id` is where it has reached. They advance independently, and
-- a tracker that merges them cannot express the ordinary case: resolved, on qa,
-- verified there, not yet promoted to staging. Every table below serves the
-- second clock or the custody chain; none of them touches the first.

-- ── Custody: which team owns this task ──────────────────────────────────────
--
-- Nullable on purpose. Intake happens before triage, so a task with no team is
-- not an error — it is THE TRIAGE QUEUE, which is the most useful list a lead
-- has and which the product currently cannot produce.
ALTER TABLE task ADD COLUMN team_id uuid REFERENCES team(id);

-- The team queue: "unassigned work owned by my team", which is what a lead
-- opens. `assignee IS NULL` is not in the index because assignment lives in
-- `task_assignee`; this covers the team half and the join does the rest.
CREATE INDEX task_team_ix ON task (workspace_id, team_id, status_id)
    WHERE deleted_at IS NULL;

-- Every hand-off, kept.
--
-- Not bookkeeping: the BOUNCE COUNT is the number that exposes a broken
-- process. A bug that has crossed between Android and Backend three times is a
-- specification problem rather than an engineering one, and no product surfaces
-- that because no product records it. `from_team_id` is null for the first
-- assignment out of triage, which is a transfer from nobody.
CREATE TABLE task_team_transfer (
    id            uuid PRIMARY KEY,
    workspace_id  uuid NOT NULL REFERENCES workspace(id) ON DELETE CASCADE,
    task_id       uuid NOT NULL REFERENCES task(id) ON DELETE CASCADE,
    from_team_id  uuid REFERENCES team(id),
    to_team_id    uuid NOT NULL REFERENCES team(id),
    moved_by      uuid NOT NULL REFERENCES user_account(id),
    moved_at      timestamptz NOT NULL DEFAULT now(),
    -- Why it moved. Free text, because "not ours, this is the API returning 500"
    -- is the sentence the receiving team needs and no enum contains it.
    note          text,
    CHECK (from_team_id IS DISTINCT FROM to_team_id)
);
CREATE INDEX task_team_transfer_task_ix ON task_team_transfer (task_id, moved_at DESC);

-- ── The second clock: environment promotion ─────────────────────────────────
--
-- `task.environment_id` stays as the CURRENT value, so every existing filter,
-- the board, and the `EnvironmentIn` grant constraint keep working untouched.
-- This is how it got there.
--
-- What it makes answerable, none of which is today:
--   what is on staging right now, and what is not yet
--   when did WR-125 reach production
--   how long does a fix take from qa to production
CREATE TABLE task_environment_promotion (
    id             uuid PRIMARY KEY,
    workspace_id   uuid NOT NULL REFERENCES workspace(id) ON DELETE CASCADE,
    task_id        uuid NOT NULL REFERENCES task(id) ON DELETE CASCADE,
    environment_id uuid NOT NULL REFERENCES project_environment(id) ON DELETE CASCADE,
    -- Null when a developer promoted it themselves at resolve time; set when it
    -- moved as part of a release. Both happen, at different moments.
    release_id     uuid,
    promoted_by    uuid NOT NULL REFERENCES user_account(id),
    promoted_at    timestamptz NOT NULL DEFAULT now()
);
-- The task's own history, newest first — the item surface.
CREATE INDEX task_env_promotion_task_ix
    ON task_environment_promotion (task_id, promoted_at DESC);
-- "What reached staging, and when" — the environment view and the flow metric.
CREATE INDEX task_env_promotion_env_ix
    ON task_environment_promotion (workspace_id, environment_id, promoted_at DESC);

-- ── Releases: a batch promotion with a name ─────────────────────────────────
--
-- Per project, because environments are. A workspace-wide release train is a
-- wider shape and is left as an open question in docs/45 rather than guessed at.
CREATE TABLE release (
    id            uuid PRIMARY KEY,
    workspace_id  uuid NOT NULL REFERENCES workspace(id) ON DELETE CASCADE,
    project_id    uuid NOT NULL REFERENCES project(id) ON DELETE CASCADE,
    name          text NOT NULL,
    note          text,
    created_by    uuid NOT NULL REFERENCES user_account(id),
    created_at    timestamptz NOT NULL DEFAULT now(),
    UNIQUE (project_id, name)
);

-- Declared after `release` exists, so the promotion row can name one. Written as
-- a separate statement rather than inline above because the tables reference
-- each other in creation order.
ALTER TABLE task_environment_promotion
    ADD CONSTRAINT task_env_promotion_release_fk
    FOREIGN KEY (release_id) REFERENCES release(id) ON DELETE SET NULL;

-- ── Verification: an outcome, not a status change ───────────────────────────
--
-- A failed verification is not "moved back to In Progress" — that is what
-- happens as a RESULT. The fact worth keeping is that it was tested on qa and
-- failed, with the evidence, because "failed verification twice on the same
-- environment" is a sentence a status column can never produce.
CREATE TYPE verification_verdict AS ENUM ('PASS', 'FAIL');

CREATE TABLE task_verification (
    id             uuid PRIMARY KEY,
    workspace_id   uuid NOT NULL REFERENCES workspace(id) ON DELETE CASCADE,
    task_id        uuid NOT NULL REFERENCES task(id) ON DELETE CASCADE,
    -- The environment it was tested on. Not nullable: a verdict without an
    -- environment is untraceable — "it works" is not a result.
    environment_id uuid NOT NULL REFERENCES project_environment(id) ON DELETE CASCADE,
    verdict        verification_verdict NOT NULL,
    verified_by    uuid NOT NULL REFERENCES user_account(id),
    verified_at    timestamptz NOT NULL DEFAULT now(),
    -- Evidence. A failing verdict without it is a message nobody can act on.
    note           text
);
CREATE INDEX task_verification_task_ix ON task_verification (task_id, verified_at DESC);

-- ── RLS ─────────────────────────────────────────────────────────────────────
--
-- Migration 0010's catalogue loop ran once, at 0010; a table created afterwards
-- has to say this itself, and tests/schema/assertions.sql §2 fails the build if
-- it does not.
--
-- NULLIF(..., '') is load-bearing for the reason 0010 gives: a transaction-local
-- set_config reverts to the empty string rather than unsetting, and casting ''
-- to uuid raises.
ALTER TABLE task_team_transfer ENABLE ROW LEVEL SECURITY;
ALTER TABLE task_team_transfer FORCE ROW LEVEL SECURITY;
CREATE POLICY task_team_transfer_tenant_isolation ON task_team_transfer
    USING (workspace_id = NULLIF(current_setting('taskforge.workspace_id', true), '')::uuid);

ALTER TABLE task_environment_promotion ENABLE ROW LEVEL SECURITY;
ALTER TABLE task_environment_promotion FORCE ROW LEVEL SECURITY;
CREATE POLICY task_env_promotion_tenant_isolation ON task_environment_promotion
    USING (workspace_id = NULLIF(current_setting('taskforge.workspace_id', true), '')::uuid);

ALTER TABLE release ENABLE ROW LEVEL SECURITY;
ALTER TABLE release FORCE ROW LEVEL SECURITY;
CREATE POLICY release_tenant_isolation ON release
    USING (workspace_id = NULLIF(current_setting('taskforge.workspace_id', true), '')::uuid);

ALTER TABLE task_verification ENABLE ROW LEVEL SECURITY;
ALTER TABLE task_verification FORCE ROW LEVEL SECURITY;
CREATE POLICY task_verification_tenant_isolation ON task_verification
    USING (workspace_id = NULLIF(current_setting('taskforge.workspace_id', true), '')::uuid);

-- ── Grants ──────────────────────────────────────────────────────────────────
--
-- The application role gets what it needs and nothing more; migration 0012's
-- rule is that a table nobody granted is a table nobody can read, which is how
-- a missing grant fails loudly at deploy rather than quietly at runtime.
GRANT SELECT, INSERT ON task_team_transfer TO taskforge_app;
GRANT SELECT, INSERT ON task_environment_promotion TO taskforge_app;
GRANT SELECT, INSERT, UPDATE ON release TO taskforge_app;
GRANT SELECT, INSERT ON task_verification TO taskforge_app;

-- No DELETE anywhere here, deliberately: a custody chain you can edit is not a
-- custody chain. A wrong transfer is corrected by transferring back, which
-- leaves both facts in the record — the same reasoning ADR-006 applies to the
-- audit trail.
