-- 0014 — The dispatcher role (C-011 runtime half, docs/25 §Dispatch).
--
-- WHY A SECOND ROLE AND NOT A FLAG ON THE FIRST
--
-- The dispatcher polls across every tenant: a background worker cannot know the
-- set of workspace ids in advance, and a per-request tenant predicate would
-- stop delivery entirely. So it needs to read `outbox_delivery` — a table with
-- row-level security and a policy — without that policy applying.
--
-- Migration 0010 exempts `outbox_event` from RLS for the same reason. That was
-- acceptable for a table holding only the immutable fact. `outbox_delivery`
-- carries the same workspace_id and the join reaches the payload, so exempting
-- the TABLE would remove tenant isolation from the request path as well.
--
-- The alternative taken here: keep the policy, and give the one process that
-- must see across tenants its own role that bypasses it. The capability then
-- belongs to a login that only the dispatcher uses, and `taskforge_app` — the
-- role every request runs as — remains fully constrained.
--
-- WHAT THIS ROLE CANNOT DO
--
-- It is granted on the two outbox tables and nothing else. BYPASSRLS is a
-- blunt instrument: it applies to every table in the database, so the grants
-- are what actually bound it. A dispatcher that is compromised can read and
-- update delivery state across tenants — which is the capability it needs and
-- the reason it is not the role serving requests — but it cannot read a task,
-- a comment, or an attachment, because it has no privilege on those tables at
-- all. Bypassing a policy on a table you cannot select from grants nothing.

-- Idempotent, exactly as migration 0012 is for taskforge_app: `deploy/` creates
-- this role in the entrypoint script (with a password from the environment)
-- before migrations run, so a bare CREATE ROLE fails a real deployment on the
-- first `up` while passing every test that starts from an empty database.
DO $$
BEGIN
    IF NOT EXISTS (SELECT 1 FROM pg_roles WHERE rolname = 'taskforge_dispatcher') THEN
        CREATE ROLE taskforge_dispatcher NOLOGIN;
    END IF;
END $$;

-- Stated rather than assumed, and stated in both directions. Without BYPASSRLS
-- the dispatcher claims nothing and reports healthy; with SUPERUSER it would
-- also ignore the REVOKEs that make audit history append-only.
ALTER ROLE taskforge_dispatcher NOSUPERUSER BYPASSRLS NOCREATEDB NOCREATEROLE;

GRANT USAGE ON SCHEMA public TO taskforge_dispatcher;

-- The claim query reads both and writes only delivery state. No INSERT on
-- either: events are written by the request path inside the producing
-- transaction (ADR-006), and delivery rows alongside them. A dispatcher that
-- could insert an outbox event could manufacture an event that never happened.
GRANT SELECT ON outbox_event TO taskforge_dispatcher;
GRANT SELECT, UPDATE ON outbox_delivery TO taskforge_dispatcher;

-- The retention sweep (docs/25: dispatched rows are removed after 7 days)
-- deletes delivery rows; the event row goes when its last delivery does.
GRANT DELETE ON outbox_delivery TO taskforge_dispatcher;
GRANT DELETE ON outbox_event TO taskforge_dispatcher;

-- NOT granted, deliberately: UPDATE on outbox_event. Its columns are the
-- immutable fact. Migration 0013 removed the only mutable ones; withholding
-- UPDATE means a future column added there cannot quietly become dispatcher
-- state again.

-- No password here. `deploy/` sets one from the environment exactly as it does
-- for taskforge_app (docs/52), and a password committed to a migration would be
-- a password in the repository.
COMMENT ON ROLE taskforge_dispatcher IS
    'Outbox dispatcher. BYPASSRLS, bounded by grants on the two outbox tables only (migration 0014).';
