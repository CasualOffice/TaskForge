-- 0010 — Row-level security as the tenancy BACKSTOP (ADR-020).
--
-- Tenancy is enforced first by the WorkspaceScope capability type, which makes
-- a missing tenant filter a compile error (docs/32). RLS sits behind it as
-- defense in depth: two independent mechanisms must both fail to leak across
-- tenants.
--
-- RLS is NOT the authorization engine. It answers "is this row in my tenant",
-- not "may this actor do this" (docs/04).
--
-- set_config('taskforge.workspace_id', ..., true) is set at connection checkout
-- and is TRANSACTION-LOCAL, so a pooled connection cannot carry one tenant's
-- setting into another's transaction — the classic pooling bug in RLS
-- deployments.

-- Policy is applied to every table carrying workspace_id, discovered from the
-- catalogue rather than a hand-maintained list. A hand-maintained list silently
-- omits new tables; this cannot.
DO $$
DECLARE t text;
BEGIN
    FOR t IN
        SELECT c.relname
          FROM pg_class c
          JOIN pg_namespace n ON n.oid = c.relnamespace
         WHERE n.nspname = 'public'
           AND c.relkind IN ('r', 'p')            -- ordinary and partitioned
           AND c.relname <> 'outbox_event'        -- see the exemption below
           AND EXISTS (
                 SELECT 1 FROM pg_attribute a
                  WHERE a.attrelid = c.oid
                    AND a.attname  = 'workspace_id'
                    AND a.attnum > 0 AND NOT a.attisdropped)
    LOOP
        EXECUTE format('ALTER TABLE %I ENABLE ROW LEVEL SECURITY', t);
        EXECUTE format('ALTER TABLE %I FORCE ROW LEVEL SECURITY', t);
        -- NULLIF(..., '') is load-bearing, not defensive style. A
        -- transaction-local set_config does not *unset* on COMMIT — it reverts
        -- the value to the empty string. Casting '' to uuid raises
        -- `invalid input syntax for type uuid`, so without NULLIF every pooled
        -- connection would start erroring after its first scoped transaction.
        -- With it, an unscoped session sees NULL, the comparison is NULL, and
        -- the query returns no rows: it fails closed, and it fails gracefully.
        EXECUTE format(
            'CREATE POLICY %I ON %I USING (workspace_id = '
            'NULLIF(current_setting(''taskforge.workspace_id'', true), '''')::uuid)',
            t || '_tenant_isolation', t);
    END LOOP;
END $$;

-- ---------------------------------------------------------------------------
-- Deliberate exemptions, each with a reason. An undocumented exemption is a
-- hole; these are decisions.
-- ---------------------------------------------------------------------------
--
-- `outbox_event` — EXEMPT. The dispatcher polls pending events **across all
--   tenants** (`FOR UPDATE SKIP LOCKED`, docs/25), so a per-request tenant
--   predicate would break delivery. It is protected instead by never being
--   reachable from a request-serving code path: only `casual-task-worker`
--   queries it, using a connection that never carries a user session.
--
-- `user_account` — no workspace_id by design; a person spans workspaces. Every
--   read path reaches it through `workspace_membership`, which IS protected
--   (docs/32 §The user_account exception).
--
-- `workspace` — the tenant root itself. Row identity *is* the tenant.
--
-- `permission` — seeded reference data, identical for every tenant.
--
-- `team_membership`, `role_permission` — reached only through `team` and `role`
--   respectively, both of which are protected. They carry no tenant data of
--   their own beyond the association.

-- ---------------------------------------------------------------------------
-- Append-only history, enforced by GRANT rather than by convention (docs/25).
-- The application role can insert and read history; there is no code path that
-- can rewrite it because the privilege does not exist.
-- ---------------------------------------------------------------------------
DO $$
BEGIN
    IF EXISTS (SELECT 1 FROM pg_roles WHERE rolname = 'taskforge_app') THEN
        REVOKE UPDATE, DELETE ON activity_event, audit_event FROM taskforge_app;
    END IF;
END $$;
