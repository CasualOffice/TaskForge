-- benchmarks/smoke-corpus.sql — a REDUCED, DISPOSABLE corpus for exercising
-- tools/casual-task-loadtest. Read this header before using any number that
-- came out of it.
--
-- THIS IS NOT THE REFERENCE CORPUS.
--
-- The reference corpus is 2,000,000 tasks / 200 projects / 500 users, generated
-- deterministically by `tools/casual-task-seed` (tracker F-006, LANDED), and is
-- what docs/30 §Reference capacity gates against. This file predates it: it
-- existed because F-007 must not claim a harness works without having run it,
-- and at the time there was nothing to run it against. It generates ~1/20th of
-- the reference task count on one workspace, with no attempt at realistic
-- distributions.
--
-- Consequences, stated plainly:
--   * Numbers measured against this corpus are NOT comparable to numbers
--     measured against the reference corpus, and must never be committed as a
--     `reference` baseline. Use `--corpus-scale smoke`.
--   * At this size most of the working set fits in shared_buffers, so index
--     scans do not pay the random I/O they will pay at 2M tasks. Read every
--     measurement as a lower bound.
--   * Distributions are uniform. Real workspaces are not, and skew is what
--     breaks plans.
--
-- KEPT, not deleted. The earlier note here said to delete this file once
-- casual-task-seed landed, on the grounds that two corpus generators is one too
-- many — which is true for new work, and `casual-task-seed --scale small` is
-- what new work should use. But `smoke-local.smoke.json` is a committed
-- measurement OF THIS CORPUS, and deleting the corpus would leave a number
-- nobody can reproduce or check. It stays as provenance for that file, and for
-- nothing else. Do not build on it.
--
-- Ids are derived from counters rather than random so a regenerated corpus is
-- byte-identical and the harness's deterministic probes pick the same rows.
--
-- Usage (as the OWNER role, not taskforge_app — RLS is bypassed for the owner
-- and this writes across the whole workspace):
--
--   psql "$OWNER_DSN" -v ON_ERROR_STOP=1 -f benchmarks/smoke-corpus.sql

\set ON_ERROR_STOP on
\timing off

BEGIN;

-- Tunables. Kept small enough to generate in well under a minute.
\set n_users     60
\set n_teams     5
\set n_projects  20
\set n_tasks     100000
\set n_statuses  5

-- ---------------------------------------------------------------------------
-- Tenancy and identity
-- ---------------------------------------------------------------------------
INSERT INTO workspace (id, name, slug)
VALUES ('00000000-0000-7000-8000-000000000001', 'Smoke', 'smoke');

INSERT INTO user_account (id, email, display_name)
SELECT ('00000001-0000-7000-8000-' || lpad(to_hex(n), 12, '0'))::uuid,
       'user' || n || '@smoke.test',
       'Smoke User ' || n
  FROM generate_series(1, :n_users) n;

INSERT INTO workspace_membership (workspace_id, user_id, member_type)
SELECT '00000000-0000-7000-8000-000000000001',
       ('00000001-0000-7000-8000-' || lpad(to_hex(n), 12, '0'))::uuid,
       'MEMBER'
  FROM generate_series(1, :n_users) n;

INSERT INTO team (id, workspace_id, name)
SELECT ('00000006-0000-7000-8000-' || lpad(to_hex(n), 12, '0'))::uuid,
       '00000000-0000-7000-8000-000000000001',
       'Team ' || n
  FROM generate_series(1, :n_teams) n;

INSERT INTO team_membership (team_id, user_id)
SELECT ('00000006-0000-7000-8000-' || lpad(to_hex(1 + n % :n_teams), 12, '0'))::uuid,
       ('00000001-0000-7000-8000-' || lpad(to_hex(n), 12, '0'))::uuid
  FROM generate_series(1, :n_users) n;

-- ---------------------------------------------------------------------------
-- Authorization. Without grants the accessible-project pre-filter in the search
-- case degenerates, so the corpus carries real role assignments.
-- ---------------------------------------------------------------------------
INSERT INTO role (id, workspace_id, name, is_template) VALUES
  ('00000005-0000-7000-8000-000000000001', '00000000-0000-7000-8000-000000000001', 'Admin',  true),
  ('00000005-0000-7000-8000-000000000002', '00000000-0000-7000-8000-000000000001', 'Member', true),
  ('00000005-0000-7000-8000-000000000003', '00000000-0000-7000-8000-000000000001', 'Viewer', true);

INSERT INTO role_permission (role_id, permission)
SELECT '00000005-0000-7000-8000-000000000001', key FROM permission;
INSERT INTO role_permission (role_id, permission)
SELECT '00000005-0000-7000-8000-000000000002', key
  FROM permission WHERE key LIKE 'task.%' OR key LIKE 'comment.%';
INSERT INTO role_permission (role_id, permission)
SELECT '00000005-0000-7000-8000-000000000003', key
  FROM permission WHERE key = 'task.read';

-- ---------------------------------------------------------------------------
-- Workflow
-- ---------------------------------------------------------------------------
INSERT INTO workflow (id, workspace_id, name, is_default)
VALUES ('00000009-0000-7000-8000-000000000001',
        '00000000-0000-7000-8000-000000000001', 'Default', true);

INSERT INTO workflow_status (id, workflow_id, workspace_id, name, state, position, is_initial)
VALUES
 ('00000004-0000-7000-8000-000000000001','00000009-0000-7000-8000-000000000001','00000000-0000-7000-8000-000000000001','Backlog','BACKLOG',1,true),
 ('00000004-0000-7000-8000-000000000002','00000009-0000-7000-8000-000000000001','00000000-0000-7000-8000-000000000001','Planned','PLANNED',2,false),
 ('00000004-0000-7000-8000-000000000003','00000009-0000-7000-8000-000000000001','00000000-0000-7000-8000-000000000001','In Progress','ACTIVE',3,false),
 ('00000004-0000-7000-8000-000000000004','00000009-0000-7000-8000-000000000001','00000000-0000-7000-8000-000000000001','Done','COMPLETED',4,false),
 ('00000004-0000-7000-8000-000000000005','00000009-0000-7000-8000-000000000001','00000000-0000-7000-8000-000000000001','Canceled','CANCELED',5,false);

-- ---------------------------------------------------------------------------
-- Projects
-- ---------------------------------------------------------------------------
INSERT INTO project (id, workspace_id, key, name, visibility, workflow_id, task_seq, created_by)
SELECT ('00000002-0000-7000-8000-' || lpad(to_hex(n), 12, '0'))::uuid,
       '00000000-0000-7000-8000-000000000001',
       'PRJ' || lpad(n::text, 2, '0'),
       'Project ' || n,
       'TEAM',
       '00000009-0000-7000-8000-000000000001',
       0,
       '00000001-0000-7000-8000-000000000001'
  FROM generate_series(1, :n_projects) n;

-- User 1 is a workspace-scoped admin; everyone else holds project-scoped
-- grants, so the accessible-project set differs per actor.
INSERT INTO role_assignment (id, workspace_id, principal_type, principal_id, role_id, scope_type, scope_id, granted_by)
VALUES ('00000008-0000-7000-8000-000000000001',
        '00000000-0000-7000-8000-000000000001', 'USER',
        '00000001-0000-7000-8000-000000000001',
        '00000005-0000-7000-8000-000000000001', 'WORKSPACE',
        '00000000-0000-7000-8000-000000000001',
        '00000001-0000-7000-8000-000000000001');

INSERT INTO role_assignment (id, workspace_id, principal_type, principal_id, role_id, scope_type, scope_id, granted_by)
SELECT ('00000008-0000-7000-8000-' || lpad(to_hex(1000 + row_number() OVER ()), 12, '0'))::uuid,
       '00000000-0000-7000-8000-000000000001',
       'USER',
       ('00000001-0000-7000-8000-' || lpad(to_hex(u), 12, '0'))::uuid,
       '00000005-0000-7000-8000-000000000002',
       'PROJECT',
       ('00000002-0000-7000-8000-' || lpad(to_hex(p), 12, '0'))::uuid,
       '00000001-0000-7000-8000-000000000001'
  FROM generate_series(2, :n_users) u,
       generate_series(1, :n_projects) p
 WHERE (u + p) % 3 = 0;

INSERT INTO project_membership (project_id, user_id, workspace_id)
SELECT ('00000002-0000-7000-8000-' || lpad(to_hex(p), 12, '0'))::uuid,
       ('00000001-0000-7000-8000-' || lpad(to_hex(u), 12, '0'))::uuid,
       '00000000-0000-7000-8000-000000000001'
  FROM generate_series(2, :n_users) u,
       generate_series(1, :n_projects) p
 WHERE (u + p) % 3 = 0;

-- ---------------------------------------------------------------------------
-- Tasks. Titles are drawn from a small vocabulary so the full-text case has
-- something to match; `position` is a zero-padded hex rank, which sorts
-- correctly as text the way ADR-013 requires.
-- ---------------------------------------------------------------------------
INSERT INTO task (id, workspace_id, project_id, number, title, description, type,
                  priority, status_id, state, reporter_id, position,
                  created_at, created_by, updated_at, due_at)
SELECT ('00000003-0000-7000-8000-' || lpad(to_hex(n), 12, '0'))::uuid,
       '00000000-0000-7000-8000-000000000001',
       ('00000002-0000-7000-8000-' || lpad(to_hex(1 + n % :n_projects), 12, '0'))::uuid,
       1 + n / :n_projects,
       (ARRAY['payment','search','import','export','session','webhook','index','digest'])[1 + n % 8]
         || ' ' ||
       (ARRAY['retry','timeout','failure','latency','backlog','migration','cleanup','audit'])[1 + (n / 8) % 8]
         || ' #' || n,
       'Reported by the smoke corpus. Reference number ' || n || '. '
         || (ARRAY['Occurs under load.','Reproduced on staging.','Needs a repro case.','Blocked on review.'])[1 + n % 4],
       (ARRAY['TASK','BUG','FEATURE','INCIDENT','REQUEST'])[1 + n % 5]::task_type,
       (ARRAY['NONE','LOW','MEDIUM','HIGH','URGENT'])[1 + n % 5]::task_priority,
       ('00000004-0000-7000-8000-' || lpad(to_hex(1 + n % :n_statuses), 12, '0'))::uuid,
       (ARRAY['BACKLOG','PLANNED','ACTIVE','COMPLETED','CANCELED'])[1 + n % :n_statuses]::task_state,
       ('00000001-0000-7000-8000-' || lpad(to_hex(1 + n % :n_users), 12, '0'))::uuid,
       lpad(to_hex(n), 10, '0'),
       TIMESTAMPTZ '2026-01-01 00:00:00Z' + (n || ' seconds')::interval,
       '00000001-0000-7000-8000-000000000001',
       TIMESTAMPTZ '2026-01-01 00:00:00Z' + (n || ' seconds')::interval,
       CASE WHEN n % 3 = 0
            THEN TIMESTAMPTZ '2026-06-01 00:00:00Z' + (n || ' minutes')::interval
       END
  FROM generate_series(1, :n_tasks) n;

UPDATE project p
   SET task_seq = c.max_number
  FROM (SELECT project_id, max(number) AS max_number FROM task GROUP BY project_id) c
 WHERE c.project_id = p.id;

-- Two thirds of tasks are assigned; one assignee each, marked primary.
INSERT INTO task_assignee (task_id, user_id, workspace_id, is_primary)
SELECT ('00000003-0000-7000-8000-' || lpad(to_hex(n), 12, '0'))::uuid,
       ('00000001-0000-7000-8000-' || lpad(to_hex(1 + n % :n_users), 12, '0'))::uuid,
       '00000000-0000-7000-8000-000000000001',
       true
  FROM generate_series(1, :n_tasks) n
 WHERE n % 3 <> 0;

-- ---------------------------------------------------------------------------
-- Search projection (docs/26 §The search projection), with the A/B/C weighting.
-- ---------------------------------------------------------------------------
INSERT INTO task_search (task_id, workspace_id, project_id, document, title_trgm, updated_at)
SELECT t.id, t.workspace_id, t.project_id,
       setweight(to_tsvector('english', t.title), 'A')
         || setweight(to_tsvector('english', coalesce(t.description, '')), 'C'),
       t.title,
       t.updated_at
  FROM task t;

-- ---------------------------------------------------------------------------
-- Activity. Three events per task so the history-tab case has a real page.
-- ---------------------------------------------------------------------------
INSERT INTO activity_event (id, workspace_id, project_id, aggregate_type, aggregate_id,
                            event_type, actor_id, changes, occurred_at)
SELECT ('00000007-0000-7000-8000-' || lpad(to_hex(n * 4 + k), 12, '0'))::uuid,
       t.workspace_id, t.project_id, 'TASK', t.id,
       (ARRAY['task.created','task.updated','task.transitioned'])[k + 1],
       t.reporter_id,
       jsonb_build_object('field', 'status', 'sequence', k),
       t.created_at + (k || ' hours')::interval
  FROM generate_series(1, :n_tasks) n
  JOIN task t ON t.id = ('00000003-0000-7000-8000-' || lpad(to_hex(n), 12, '0'))::uuid,
       generate_series(0, 2) k;

COMMIT;

-- Statistics must be current or the planner picks shapes the product would
-- never see, and the harness would measure a plan nobody ships.
ANALYZE;

SELECT 'tasks'    AS entity, count(*) FROM task
UNION ALL SELECT 'projects',  count(*) FROM project
UNION ALL SELECT 'users',     count(*) FROM user_account
UNION ALL SELECT 'search',    count(*) FROM task_search
UNION ALL SELECT 'activity',  count(*) FROM activity_event
UNION ALL SELECT 'grants',    count(*) FROM role_assignment;
