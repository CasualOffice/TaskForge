-- Deterministic planning corpus for the EXPLAIN gate (scripts/verify-queries.sh).
--
-- WHY THIS FILE EXISTS
--
-- An EXPLAIN assertion against an empty table proves nothing: PostgreSQL picks a
-- sequential scan for a zero-page relation no matter how many indexes exist, so a
-- gate that "passes" there measures nothing at all. This corpus exists to make
-- the planner's choice meaningful — every table the gate asserts on is large
-- enough, and skewed enough, that an index is genuinely the cheaper plan.
--
-- IT IS NOT THE REFERENCE CORPUS. docs/26 §Acceptance gates calls for 2M tasks /
-- 200 projects / 500 users for the *latency* gates. This is ~109k tasks: two
-- orders of magnitude smaller, sized so the gate runs in under a minute on every
-- pull request. The cost of that choice, stated plainly: this corpus proves
-- **plan shape**, not latency. A query that is index-served here can still be too
-- slow at reference scale; F-007 (latency gate) is what catches that, and it does
-- not exist yet.
--
-- Determinism: every id is a function of its ordinal and every timestamp is an
-- offset from a fixed anchor instant. There is no random() and no now(), so two
-- runs produce identical data and a plan change is always a code change.
--
-- Cardinality shape, and why each number is what it is:
--
--   100 workspaces      — workspace_id must be SELECTIVE. With two workspaces the
--                         planner estimates `workspace_id = $1` at 50% and
--                         rationally sequential-scans; the gate would then fail
--                         for a reason that does not exist in production.
--   436 projects        — 4 per workspace, except the focus tenant's 40.
--   500 users           — shared across workspaces; user_account is global.
--   109k tasks          — 250 per project, so one board column is ~40 rows.
--   ~210k task_tag      — the reverse-lookup direction (docs/26 §task_tag).
--   ~218k activity      — a task's history tab needs a haystack around it.
--   109k notification   — 87% read, so notification_unread_ix is the small one.
--   109k outbox_event   — 97% dispatched, so outbox_pending_ix is genuinely tiny
--                         and preferring it is the planner's own conclusion.
--
-- Run as the OWNER. RLS is FORCEd on these tables (migration 0010), and seeding
-- is not a request path, so it does not carry a tenant scope.

\set ON_ERROR_STOP on

\set n_workspaces 100
\set n_users 500
-- Projects per workspace: 4 for the ordinary tenants, 40 for the focus tenant
-- (see tf_seed_project_count). :focus_projects is also the id stride, so project
-- ordinals stay unique and derivable despite the variable count.
\set projects_per_ws 4
\set focus_projects 40
\set tasks_per_project 250
\set tags_per_ws 20

-- A fixed instant, so "overdue" and "upcoming" windows are stable across runs.
-- The query catalogue anchors to this same literal instead of now().
\set epoch '''2026-01-01 00:00:00+00'''

-- Id scheme: a kind byte plus a zero-padded ordinal. Readable in an EXPLAIN,
-- ordered, and collision-free without a generator or a sequence.
CREATE OR REPLACE FUNCTION tf_seed_id(kind int, n bigint) RETURNS uuid
    LANGUAGE sql IMMUTABLE AS
$$ SELECT (lpad(to_hex(kind), 2, '0') || '000000-0000-7000-8000-'
           || lpad(to_hex(n), 12, '0'))::uuid $$;

-- The focus tenant is deliberately larger than the rest: 40 projects and 10,000
-- tasks against 4 and 1,000 elsewhere. A four-project accessible set makes the
-- permission pre-filter so selective that no search plan is under any pressure,
-- and the full-text assertions would be asserting nothing about full text. Forty
-- is also what docs/26 §Permission filtering calls the typical cardinality of
-- `accessible_projects` ("tens").
--
-- It turned out not to be enough to get the GIN index chosen, for a reason that
-- has nothing to do with corpus size — see the note in
-- queries/11-search-fulltext-ranked.sql. The larger tenant is kept because it is
-- the realistic shape regardless.
CREATE OR REPLACE FUNCTION tf_seed_project_count(w int) RETURNS int
    LANGUAGE sql IMMUTABLE AS $$ SELECT CASE WHEN w = 1 THEN 40 ELSE 4 END $$;

-- The inverse, so derived tables can be seeded from `task` by ordinal without a
-- second generate_series that would have to stay in sync with the first.
CREATE OR REPLACE FUNCTION tf_seed_ordinal(u uuid) RETURNS bigint
    LANGUAGE sql IMMUTABLE AS
$$ SELECT ('x' || lpad(right(replace(u::text, '-', ''), 12), 16, '0'))::bit(64)::bigint $$;

-- ---------------------------------------------------------------------------
-- Tenancy and identity
-- ---------------------------------------------------------------------------
INSERT INTO workspace (id, name, slug)
SELECT tf_seed_id(1, w), 'Workspace ' || w, 'ws-' || w
  FROM generate_series(1, :n_workspaces) w;

INSERT INTO user_account (id, email, display_name)
SELECT tf_seed_id(2, u), 'user' || u || '@example.test', 'User ' || u
  FROM generate_series(1, :n_users) u;

-- 20 members per workspace, users reused across workspaces: a person legitimately
-- spans tenants (docs/32 §The user_account exception).
INSERT INTO workspace_membership (workspace_id, user_id, member_type)
SELECT tf_seed_id(1, w), tf_seed_id(2, (w * 7 + k) % :n_users + 1),
       CASE WHEN k = 20 THEN 'GUEST' ELSE 'MEMBER' END
  FROM generate_series(1, :n_workspaces) w, generate_series(1, 20) k;

INSERT INTO team (id, workspace_id, name)
SELECT tf_seed_id(6, (w - 1) * 3 + t), tf_seed_id(1, w), 'Team ' || t
  FROM generate_series(1, :n_workspaces) w, generate_series(1, 3) t;

INSERT INTO team_membership (team_id, user_id)
SELECT tf_seed_id(6, (w - 1) * 3 + t), tf_seed_id(2, (w * 7 + t * 2 + k) % :n_users + 1)
  FROM generate_series(1, :n_workspaces) w, generate_series(1, 3) t, generate_series(1, 5) k
 ON CONFLICT DO NOTHING;

-- ---------------------------------------------------------------------------
-- Workflow, projects, milestones, tags
-- ---------------------------------------------------------------------------
INSERT INTO workflow (id, workspace_id, name, is_default)
SELECT tf_seed_id(8, w), tf_seed_id(1, w), 'Default', true
  FROM generate_series(1, :n_workspaces) w;

-- Six statuses per workflow. `state` is the permanent contract (docs/23) and the
-- task rows below derive theirs from the status they point at — the invariant
-- that lets My Work filter on state without a join.
INSERT INTO workflow_status (id, workflow_id, workspace_id, name, state, position, is_initial)
SELECT tf_seed_id(9, (w - 1) * 6 + s), tf_seed_id(8, w), tf_seed_id(1, w),
       (ARRAY['Backlog','Planned','In Progress','In Review','Done','Canceled'])[s],
       (ARRAY['BACKLOG','PLANNED','ACTIVE','ACTIVE','COMPLETED','CANCELED'])[s]::task_state,
       s, s = 1
  FROM generate_series(1, :n_workspaces) w, generate_series(1, 6) s;

INSERT INTO project (id, workspace_id, team_id, key, name, workflow_id, created_by, task_seq)
SELECT tf_seed_id(3, (w - 1) * :focus_projects + p),
       tf_seed_id(1, w),
       tf_seed_id(6, (w - 1) * 3 + (p % 3 + 1)),
       'P' || lpad(((w - 1) * :focus_projects + p)::text, 5, '0'),
       'Project ' || p,
       tf_seed_id(8, w),
       tf_seed_id(2, (w * 7 + 1) % :n_users + 1),
       :tasks_per_project
  FROM generate_series(1, :n_workspaces) w, generate_series(1, tf_seed_project_count(w)) p;

INSERT INTO project_membership (project_id, user_id, workspace_id)
SELECT tf_seed_id(3, (w - 1) * :focus_projects + p),
       tf_seed_id(2, (w * 7 + k) % :n_users + 1),
       tf_seed_id(1, w)
  FROM generate_series(1, :n_workspaces) w,
       generate_series(1, tf_seed_project_count(w)) p,
       generate_series(1, 10) k
 ON CONFLICT DO NOTHING;

INSERT INTO project_environment (id, project_id, workspace_id, name, position)
SELECT tf_seed_id(11, ((w - 1) * :focus_projects + p - 1) * 2 + e),
       tf_seed_id(3, (w - 1) * :focus_projects + p), tf_seed_id(1, w),
       (ARRAY['staging','production'])[e], e
  FROM generate_series(1, :n_workspaces) w,
       generate_series(1, tf_seed_project_count(w)) p, generate_series(1, 2) e;

INSERT INTO milestone (id, workspace_id, project_id, name, due_at)
SELECT tf_seed_id(10, ((w - 1) * :focus_projects + p - 1) * 2 + m),
       tf_seed_id(1, w), tf_seed_id(3, (w - 1) * :focus_projects + p),
       'M' || m, :epoch::timestamptz + (m * 30 || ' days')::interval
  FROM generate_series(1, :n_workspaces) w,
       generate_series(1, tf_seed_project_count(w)) p, generate_series(1, 2) m;

INSERT INTO tag (id, workspace_id, project_id, name)
SELECT tf_seed_id(5, (w - 1) * :tags_per_ws + g), tf_seed_id(1, w), NULL, 'tag-' || g
  FROM generate_series(1, :n_workspaces) w, generate_series(1, :tags_per_ws) g;

-- ---------------------------------------------------------------------------
-- Authorization. role_assignment is read on every request (docs/04 §Caching),
-- so it is seeded at a realistic width: a workspace-scope grant per member plus
-- project- and team-scope grants. A one-row-per-workspace table would make the
-- resolver assertion vacuous.
-- ---------------------------------------------------------------------------
INSERT INTO role (id, workspace_id, name, is_template)
SELECT tf_seed_id(7, (w - 1) * 4 + r), tf_seed_id(1, w),
       (ARRAY['Workspace Admin','Project Manager','Member','Guest'])[r], true
  FROM generate_series(1, :n_workspaces) w, generate_series(1, 4) r;

INSERT INTO role_permission (role_id, permission)
SELECT tf_seed_id(7, (w - 1) * 4 + r), p.key
  FROM generate_series(1, :n_workspaces) w, generate_series(1, 4) r,
       (SELECT key, row_number() OVER (ORDER BY key) rn FROM permission) p
 WHERE p.rn <= r * 7;

INSERT INTO role_assignment (id, workspace_id, principal_type, principal_id, role_id,
                             scope_type, scope_id, granted_by)
SELECT tf_seed_id(4, (w - 1) * 20 + k), tf_seed_id(1, w), 'USER',
       tf_seed_id(2, (w * 7 + k) % :n_users + 1),
       tf_seed_id(7, (w - 1) * 4 + (k % 4 + 1)),
       'WORKSPACE', tf_seed_id(1, w),
       tf_seed_id(2, (w * 7 + 1) % :n_users + 1)
  FROM generate_series(1, :n_workspaces) w, generate_series(1, 20) k
 ON CONFLICT DO NOTHING;

INSERT INTO role_assignment (id, workspace_id, principal_type, principal_id, role_id,
                             scope_type, scope_id, granted_by)
SELECT tf_seed_id(4, 100000 + ((w - 1) * :focus_projects + p - 1) * 10 + k),
       tf_seed_id(1, w), 'USER',
       tf_seed_id(2, (w * 7 + k) % :n_users + 1),
       tf_seed_id(7, (w - 1) * 4 + 2),
       'PROJECT', tf_seed_id(3, (w - 1) * :focus_projects + p),
       tf_seed_id(2, (w * 7 + 1) % :n_users + 1)
  FROM generate_series(1, :n_workspaces) w,
       generate_series(1, tf_seed_project_count(w)) p, generate_series(1, 10) k
 ON CONFLICT DO NOTHING;

-- Team-scope grants, so the resolver's principal expansion has something to find.
INSERT INTO role_assignment (id, workspace_id, principal_type, principal_id, role_id,
                             scope_type, scope_id, granted_by)
SELECT tf_seed_id(4, 900000 + (w - 1) * 3 + t), tf_seed_id(1, w), 'TEAM',
       tf_seed_id(6, (w - 1) * 3 + t), tf_seed_id(7, (w - 1) * 4 + 3),
       'TEAM', tf_seed_id(6, (w - 1) * 3 + t),
       tf_seed_id(2, (w * 7 + 1) % :n_users + 1)
  FROM generate_series(1, :n_workspaces) w, generate_series(1, 3) t
 ON CONFLICT DO NOTHING;

-- ---------------------------------------------------------------------------
-- Tasks — the hot table. States are skewed the way a live tracker's are (most
-- work is not ACTIVE) and 2% are soft-deleted, so the `WHERE deleted_at IS NULL`
-- partial indexes are measurably narrower than the table rather than identical
-- to it.
-- ---------------------------------------------------------------------------
INSERT INTO task (id, workspace_id, project_id, number, title, description, type, priority,
                  status_id, state, reporter_id, environment_id, milestone_id,
                  due_at, position, created_at, created_by, updated_at, deleted_at)
SELECT tf_seed_id(20, g.gid),
       tf_seed_id(1, w),
       tf_seed_id(3, pr.pid),
       n,
       'Task ' || n || ' of project ' || pr.pid,
       CASE WHEN n % 3 = 0 THEN NULL ELSE 'Description body for task ' || g.gid END,
       (ARRAY['TASK','BUG','FEATURE','INCIDENT','REQUEST'])[n % 5 + 1]::task_type,
       (ARRAY['NONE','LOW','MEDIUM','HIGH','URGENT'])[n % 5 + 1]::task_priority,
       tf_seed_id(9, (w - 1) * 6 + st.s),
       (ARRAY['BACKLOG','PLANNED','ACTIVE','ACTIVE','COMPLETED','CANCELED'])[st.s]::task_state,
       tf_seed_id(2, (w * 7 + n % 20 + 1) % :n_users + 1),
       CASE WHEN n % 4 = 0 THEN tf_seed_id(11, (pr.pid - 1) * 2 + (n % 2 + 1)) END,
       CASE WHEN n % 5 = 0 THEN tf_seed_id(10, (pr.pid - 1) * 2 + (n % 2 + 1)) END,
       -- A third have no due date; the rest straddle the anchor instant, so
       -- "overdue" and "upcoming" are both non-empty and neither is a majority.
       CASE WHEN n % 3 = 0 THEN NULL
            ELSE :epoch::timestamptz + ((n % 120) - 60 || ' days')::interval END,
       lpad(to_hex(n * 64), 8, '0'),                       -- lexicographic rank (ADR-013)
       :epoch::timestamptz - ((n % 400) || ' days')::interval,
       tf_seed_id(2, (w * 7 + n % 20 + 1) % :n_users + 1),
       :epoch::timestamptz - ((n % 90) || ' hours')::interval,
       CASE WHEN n % 50 = 0 THEN :epoch::timestamptz END
  FROM generate_series(1, :n_workspaces) w,
       generate_series(1, tf_seed_project_count(w)) p,
       generate_series(1, :tasks_per_project) n,
       LATERAL (SELECT (w - 1) * :focus_projects + p) pr(pid),
       LATERAL (SELECT (pr.pid - 1) * :tasks_per_project + n) g(gid),
       LATERAL (SELECT CASE WHEN n % 10 < 3 THEN 1 WHEN n % 10 < 5 THEN 2
                            WHEN n % 10 < 7 THEN 3 WHEN n % 10 < 8 THEN 4
                            WHEN n % 10 < 9 THEN 5 ELSE 6 END) st(s);

-- 80% assigned. "Unassigned" is a real built-in view (docs/27 §Built-in views)
-- and must not be an empty bucket.
INSERT INTO task_assignee (task_id, user_id, workspace_id, is_primary)
SELECT t.id, tf_seed_id(2, (tf_seed_ordinal(t.id) * 13) % :n_users + 1), t.workspace_id, true
  FROM task t
 WHERE tf_seed_ordinal(t.id) % 5 <> 0;

-- The probe user for the My Work queries. Seeded explicitly so "assigned to me"
-- has the ~140-row cardinality a real person's queue has inside one workspace,
-- rather than whatever the modulo above happened to produce.
INSERT INTO task_assignee (task_id, user_id, workspace_id, is_primary)
SELECT t.id, tf_seed_id(2, 8), t.workspace_id, false
  FROM task t
 WHERE t.workspace_id = tf_seed_id(1, 1) AND t.number % 7 = 0
 ON CONFLICT DO NOTHING;

-- Two tags per task, always from the task's own workspace — a cross-workspace
-- tag_id would be unreachable under RLS and would silently empty the reverse
-- lookup.
INSERT INTO task_tag (task_id, tag_id, workspace_id)
SELECT t.id,
       tf_seed_id(5, (tf_seed_ordinal(t.workspace_id) - 1) * :tags_per_ws
                     + (tf_seed_ordinal(t.id) * (7 * k)) % :tags_per_ws + 1),
       t.workspace_id
  FROM task t, generate_series(1, 2) k
 ON CONFLICT DO NOTHING;

INSERT INTO task_dependency (from_task_id, to_task_id, workspace_id)
SELECT t.id, o.id, t.workspace_id
  FROM task t
  JOIN task o ON o.project_id = t.project_id AND o.number = t.number - 1
 WHERE t.number % 11 = 0
 ON CONFLICT DO NOTHING;

INSERT INTO comment (id, workspace_id, task_id, author_id, body, created_at)
SELECT tf_seed_id(12, tf_seed_ordinal(t.id) * 2 + k),
       t.workspace_id, t.id,
       tf_seed_id(2, (tf_seed_ordinal(t.id) * 11) % :n_users + 1),
       'Comment ' || k || ' on task ' || t.number,
       t.created_at + (k || ' hours')::interval
  FROM task t, generate_series(1, 2) k
 WHERE t.number % 2 = 0;

-- ---------------------------------------------------------------------------
-- The search projection (docs/26 §The search projection), weighted exactly as
-- the outbox worker will write it — a differently-weighted document produces a
-- different GIN plan, so an approximation here would gate the wrong thing.
-- ---------------------------------------------------------------------------
INSERT INTO task_search (task_id, workspace_id, project_id, document, title_trgm, updated_at)
SELECT t.id, t.workspace_id, t.project_id,
       setweight(to_tsvector('english', p.key || '-' || t.number || ' ' || t.title), 'A')
       || setweight(to_tsvector('english',
              -- A rare term, one task per project: the selective probe that
              -- common board vocabulary cannot provide.
              CASE WHEN t.number % 250 = 7 THEN 'zylophage' ELSE '' END), 'B')
       || setweight(to_tsvector('english', coalesce(t.description, '')), 'C'),
       t.title, t.updated_at
  FROM task t JOIN project p ON p.id = t.project_id;

-- ---------------------------------------------------------------------------
-- History, notifications, outbox
-- ---------------------------------------------------------------------------
INSERT INTO activity_event (id, workspace_id, project_id, aggregate_type, aggregate_id,
                            event_type, actor_id, changes, occurred_at)
SELECT tf_seed_id(13, tf_seed_ordinal(t.id) * 2 + k),
       t.workspace_id, t.project_id, 'task', t.id,
       (ARRAY['task.created','task.transitioned'])[k],
       tf_seed_id(2, (tf_seed_ordinal(t.id) * 11) % :n_users + 1),
       '{"field":"status"}'::jsonb,
       t.created_at + (k || ' minutes')::interval
  FROM task t, generate_series(1, 2) k;

INSERT INTO audit_event (id, workspace_id, event_type, actor_id, actor_type,
                         target_type, target_id, occurred_at)
SELECT tf_seed_id(14, tf_seed_ordinal(t.id)),
       t.workspace_id,
       (ARRAY['task.created','role.granted','token.issued'])[t.number % 3 + 1],
       tf_seed_id(2, (tf_seed_ordinal(t.id) * 11) % :n_users + 1),
       'USER', 'task', t.id, t.created_at
  FROM task t;

-- 87% already read, so notification_unread_ix stays much smaller than the table,
-- which is the entire reason it is partial (docs/26 §Outbox & workers).
INSERT INTO notification (id, workspace_id, user_id, event_type, reason, aggregate_id,
                          payload, created_at, read_at)
SELECT tf_seed_id(15, tf_seed_ordinal(t.id)),
       t.workspace_id,
       tf_seed_id(2, (tf_seed_ordinal(t.id) * 13) % :n_users + 1),
       'task.assigned', 'ASSIGNEE', t.id, '{}'::jsonb, t.updated_at,
       CASE WHEN t.number % 8 <> 0 THEN t.updated_at + interval '1 hour' END
  FROM task t;

INSERT INTO outbox_event (id, workspace_id, event_type, aggregate_type, aggregate_id,
                          payload, schema_version, created_at, dispatched_at, attempts)
SELECT tf_seed_id(16, tf_seed_ordinal(t.id)),
       t.workspace_id, 'task.created', 'task', t.id, '{}'::jsonb, 1, t.created_at,
       CASE WHEN t.number % 32 <> 0 THEN t.created_at + interval '1 second' END,
       CASE WHEN t.number % 512 = 0 THEN 7 ELSE 0 END
  FROM task t;

INSERT INTO saved_view (id, workspace_id, project_id, owner_id, name, filter)
SELECT tf_seed_id(17, (w - 1) * 5 + v), tf_seed_id(1, w), NULL,
       tf_seed_id(2, (w * 7 + v) % :n_users + 1), 'View ' || v,
       '{"op":"and","clauses":[]}'::jsonb
  FROM generate_series(1, :n_workspaces) w, generate_series(1, 5) v;

-- Statistics are the whole ballgame. Without ANALYZE the planner works from
-- default estimates, and its choices then say nothing about these indexes.
ANALYZE;
