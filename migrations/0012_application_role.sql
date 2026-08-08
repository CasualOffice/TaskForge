-- 0012 — The application role. See docs/32-TENANCY-AND-ISOLATION.md, docs/25.
--
-- WHY THIS MIGRATION EXISTS
--
-- Row-level security (0010) and append-only history (docs/25) are both enforced
-- through role privileges, and **neither works when the application connects as
-- a superuser**:
--
--   * Superusers and BYPASSRLS roles ignore every RLS policy unconditionally.
--     `FORCE ROW LEVEL SECURITY` forces policies for the table *owner*; it does
--     not, and cannot, constrain a superuser.
--   * `REVOKE UPDATE, DELETE ON activity_event` has no effect on a superuser
--     either — the privilege system does not apply to them.
--
-- So the two mechanisms that make cross-tenant leakage and history tampering
-- structurally impossible are inert unless the application connects as an
-- ordinary, non-superuser role. That role is created here.
--
-- DEPLOYMENT
--
--   The role is created NOLOGIN with no password. The deployment assigns
--   credentials and DATABASE_URL must use them:
--
--       ALTER ROLE taskforge_app WITH LOGIN PASSWORD '...';
--
--   Migrations and the retention worker run as the owner/superuser; the
--   request-serving application must never do so. This is asserted by a startup
--   check (see `docs/48-DEPLOYMENT-PROFILES.md`): the API refuses to start if
--   `current_setting('is_superuser')` is on.

DO $$
BEGIN
    IF NOT EXISTS (SELECT 1 FROM pg_roles WHERE rolname = 'taskforge_app') THEN
        CREATE ROLE taskforge_app NOLOGIN;
    END IF;
END $$;

-- Explicitly NOT a superuser and explicitly subject to RLS. Stated rather than
-- assumed, because both default to the right thing today and a future
-- `CREATE ROLE ... SUPERUSER` elsewhere would silently disable every policy.
ALTER ROLE taskforge_app NOSUPERUSER NOBYPASSRLS NOCREATEDB NOCREATEROLE;

GRANT USAGE ON SCHEMA public TO taskforge_app;

-- Ordinary tenant data: full DML, constrained by RLS.
GRANT SELECT, INSERT, UPDATE, DELETE ON ALL TABLES IN SCHEMA public TO taskforge_app;
GRANT USAGE, SELECT ON ALL SEQUENCES IN SCHEMA public TO taskforge_app;

-- History is append-only, enforced by the absence of the privilege rather than
-- by application discipline (docs/25). There is no code path that can rewrite
-- activity or audit because the permission does not exist.
REVOKE UPDATE, DELETE ON activity_event, audit_event FROM taskforge_app;

-- Reference data is read-only to the application; it changes by migration.
REVOKE INSERT, UPDATE, DELETE ON permission FROM taskforge_app;

-- New tables added by later migrations inherit the same grants, so a future
-- migration cannot accidentally create a table the application cannot reach —
-- or, worse, one it can rewrite history through.
ALTER DEFAULT PRIVILEGES IN SCHEMA public
    GRANT SELECT, INSERT, UPDATE, DELETE ON TABLES TO taskforge_app;
ALTER DEFAULT PRIVILEGES IN SCHEMA public
    GRANT USAGE, SELECT ON SEQUENCES TO taskforge_app;
