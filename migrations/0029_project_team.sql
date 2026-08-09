-- 0029 — A project involves many teams, not one.
-- See docs/03-DOMAIN-MODEL.md §"Teams on a project — many, not one", docs/04.
--
-- `project.team_id` was a single nullable FK. Real work does not fit that: a
-- platform team and a product team share a service, and a QA team joins for a
-- release. One team per project forces the second into `WORKSPACE` visibility —
-- which shows the project to everyone — or into per-person grants, which is
-- exactly the administration the role model exists to avoid.
--
-- The scope chain widens and stays additive. docs/04 makes a task's applicable
-- scope set `{W, T, P, E}`; it becomes `{W, T₁…Tₙ, P, E}`. A grant scoped to
-- ANY of the project's teams reaches the task. No combining rule changes, and
-- the single-team case behaves exactly as it did.
--
-- # Expand, backfill, contract (docs/52 §Upgrades)
--
-- This migration is the expand and the backfill. The contract — dropping
-- `project.team_id` — is a LATER migration, deliberately: an instance running
-- the previous build against this schema must keep working while the rollout
-- completes, and it still reads that column. This build stops reading it; the
-- next one drops it. Tracked as C-019b in docs/14.

CREATE TABLE project_team (
    workspace_id  uuid NOT NULL REFERENCES workspace(id) ON DELETE CASCADE,
    project_id    uuid NOT NULL REFERENCES project(id) ON DELETE CASCADE,
    team_id       uuid NOT NULL REFERENCES team(id) ON DELETE CASCADE,
    added_at      timestamptz NOT NULL DEFAULT now(),
    added_by      uuid REFERENCES user_account(id),
    -- The natural key. A team is on a project once or not at all; there is no
    -- second row to disambiguate, so `ON CONFLICT DO NOTHING` is the whole
    -- idempotency story for "add this team again".
    PRIMARY KEY (project_id, team_id)
);

-- The PK serves project → teams (the visibility predicate, the project view).
-- This one serves team → projects, which is the team view's list of the work a
-- team is on, and which the PK cannot answer without a scan.
CREATE INDEX project_team_team_ix ON project_team (team_id, workspace_id);

-- RLS, like every other tenant table. Migration 0010's catalogue loop ran once,
-- at 0010; a table created afterwards has to say this itself, and
-- tests/schema/assertions.sql §2 fails the build if it does not.
--
-- NULLIF(..., '') is load-bearing for the same reason 0010 gives: a
-- transaction-local set_config reverts to the empty string rather than
-- unsetting, and casting '' to uuid raises.
ALTER TABLE project_team ENABLE ROW LEVEL SECURITY;
ALTER TABLE project_team FORCE ROW LEVEL SECURITY;
CREATE POLICY project_team_tenant_isolation ON project_team
    USING (workspace_id = NULLIF(current_setting('taskforge.workspace_id', true), '')::uuid);

-- The backfill. Every project that named a team keeps it, so no project loses
-- reach at the moment this deploys — which is the property that makes the
-- widening additive in practice and not only in theory.
INSERT INTO project_team (workspace_id, project_id, team_id)
SELECT p.workspace_id, p.id, p.team_id
  FROM project p
 WHERE p.team_id IS NOT NULL
ON CONFLICT (project_id, team_id) DO NOTHING;

-- Named, so that anything still reading it during the rollout can be found and
-- so the drop is not mistaken for a data loss.
COMMENT ON COLUMN project.team_id IS
    'DEPRECATED (migration 0029): superseded by project_team. Backfilled into '
    'that table and no longer read by the application. Dropped by a later '
    'migration once no running build reads it.';
