-- Schema invariant assertions, run as the OWNER after migrations.
-- See docs/22-DATABASE-SCHEMA.md, docs/26-SEARCH-INDEXING-AND-QUERY.md.
--
-- Every assertion RAISEs on failure, so a non-zero psql exit code means a real
-- violation. These check structure; tenant isolation and append-only need a
-- second connection as taskforge_app and live in the driver script.

\set ON_ERROR_STOP on

-- --------------------------------------------------------------------------
-- 1. Every tenant table carries workspace_id.
--    docs/32: "no query, cache key, object key, index document, or background
--    job can address data without a workspace scope."
-- --------------------------------------------------------------------------
DO $$
DECLARE missing text[];
BEGIN
    SELECT array_agg(c.relname ORDER BY c.relname) INTO missing
      FROM pg_class c JOIN pg_namespace n ON n.oid = c.relnamespace
     WHERE n.nspname = 'public' AND c.relkind IN ('r','p')
       AND c.relname NOT LIKE '%_default'
       -- Documented exemptions (docs/32, migration 0010).
       --
       -- The identity tables (migration 0016) are exempt for one reason,
       -- stated once: a session, a password, an MFA factor and a recovery code
       -- belong to a PERSON, not to a tenant — which is why `user_account`
       -- itself is already on this list. They fall outside migration 0010's
       -- catalogue loop by construction rather than by decision, so they are
       -- named here explicitly. A new table joining this list without a reason
       -- beside it is the failure this comment exists to prevent.
       AND c.relname NOT IN ('workspace','user_account','permission',
                             'team_membership','role_permission',
                             'user_credential','session','mfa_factor',
                             'recovery_code','password_reset_token','auth_event')
       AND NOT EXISTS (SELECT 1 FROM pg_attribute a
                        WHERE a.attrelid = c.oid AND a.attname = 'workspace_id'
                          AND a.attnum > 0 AND NOT a.attisdropped);
    IF missing IS NOT NULL THEN
        RAISE EXCEPTION 'tenant tables without workspace_id: %', missing;
    END IF;
END $$;

-- --------------------------------------------------------------------------
-- 2. Every table with workspace_id has an RLS policy.
--    outbox_event is the single documented exemption: the dispatcher polls
--    across tenants and is never reachable from a request path (docs/25).
-- --------------------------------------------------------------------------
DO $$
DECLARE unprotected text[];
BEGIN
    SELECT array_agg(c.relname ORDER BY c.relname) INTO unprotected
      FROM pg_class c JOIN pg_namespace n ON n.oid = c.relnamespace
     WHERE n.nspname = 'public' AND c.relkind IN ('r','p')
       AND c.relname <> 'outbox_event'
       AND NOT c.relrowsecurity
       AND EXISTS (SELECT 1 FROM pg_attribute a
                    WHERE a.attrelid = c.oid AND a.attname = 'workspace_id'
                      AND a.attnum > 0 AND NOT a.attisdropped);
    IF unprotected IS NOT NULL THEN
        RAISE EXCEPTION 'workspace_id tables without RLS: %', unprotected;
    END IF;
END $$;

-- --------------------------------------------------------------------------
-- 3. RLS is FORCEd, so the table owner is subject to it too. Without FORCE,
--    policies silently do nothing for the owner.
-- --------------------------------------------------------------------------
DO $$
DECLARE unforced text[];
BEGIN
    SELECT array_agg(c.relname ORDER BY c.relname) INTO unforced
      FROM pg_class c JOIN pg_namespace n ON n.oid = c.relnamespace
     WHERE n.nspname = 'public' AND c.relrowsecurity AND NOT c.relforcerowsecurity;
    IF unforced IS NOT NULL THEN
        RAISE EXCEPTION 'RLS enabled but not FORCEd: %', unforced;
    END IF;
END $$;

-- --------------------------------------------------------------------------
-- 4. No RLS policy casts the setting directly to uuid.
--    A bare current_setting(...)::uuid raises `invalid input syntax for type
--    uuid` once a transaction-local setting has reverted to '', so every pooled
--    connection would start erroring after its first scoped transaction.
--    NULLIF(..., '') makes it fail closed instead of failing loudly.
-- --------------------------------------------------------------------------
DO $$
DECLARE bad text[];
BEGIN
    SELECT array_agg(tablename || '.' || policyname) INTO bad
      FROM pg_policies
     WHERE schemaname = 'public'
       AND qual LIKE '%current_setting%'
       AND qual NOT LIKE '%NULLIF%';
    IF bad IS NOT NULL THEN
        RAISE EXCEPTION 'RLS policy casts current_setting without NULLIF: %', bad;
    END IF;
END $$;

-- --------------------------------------------------------------------------
-- 5. The task index inventory from docs/26 exists.
--    A filter or sort field without its index is the failure this gate exists
--    to prevent (ADR-011).
-- --------------------------------------------------------------------------
DO $$
DECLARE required text[] := ARRAY[
    'task_board_ix','task_list_ix','task_mywork_ix','task_reporter_ix',
    'task_parent_ix','task_milestone_ix','task_env_ix','task_due_ix',
    'task_type_prio_ix','task_updated_brin','task_assignee_user_ix',
    'task_tag_rev_ix','task_dependency_rev_ix','task_search_gin',
    'task_search_trgm','task_search_scope_ix','role_assignment_lookup_ix',
    'role_assignment_scope_ix','project_membership_user_ix',
    -- outbox_delivery_pending_ix replaces outbox_pending_ix: migration 0013
    -- moved delivery state off outbox_event onto (event, consumer), so the
    -- index the dispatcher's claim query needs is on the delivery table.
    -- outbox_event_aggregate_ix (migration 0018) is what answers the claim
    -- query's per-aggregate ordering anti-join. Dropping it does not produce a
    -- sequential scan in the full claim plan — the planner falls back to hashing
    -- every pending delivery — so the EXPLAIN gate alone would not miss it here.
    'team_membership_user_ix','outbox_delivery_pending_ix','outbox_event_aggregate_ix',
    'notification_unread_ix','comment_task_ix','attachment_task_ix',
    -- C-006: the project list's keyset order, and the guard that keeps a
    -- workspace from ending up with two default workflows (migration 0019).
    'project_list_ix','workflow_default_uq'];
    missing text[];
BEGIN
    SELECT array_agg(r) INTO missing
      FROM unnest(required) r
     WHERE NOT EXISTS (SELECT 1 FROM pg_indexes
                        WHERE schemaname = 'public' AND indexname = r);
    IF missing IS NOT NULL THEN
        RAISE EXCEPTION 'indexes from docs/26 missing: %', missing;
    END IF;
END $$;

-- --------------------------------------------------------------------------
-- 6. The five states are exactly the five states (docs/23, ADR-002).
-- --------------------------------------------------------------------------
DO $$
DECLARE states text[];
BEGIN
    SELECT array_agg(e.enumlabel ORDER BY e.enumsortorder) INTO states
      FROM pg_enum e JOIN pg_type t ON t.oid = e.enumtypid
     WHERE t.typname = 'task_state';
    IF states <> ARRAY['BACKLOG','PLANNED','ACTIVE','COMPLETED','CANCELED'] THEN
        RAISE EXCEPTION 'task_state changed: % — this is a breaking API change', states;
    END IF;
END $$;

-- --------------------------------------------------------------------------
-- 7. The application role exists, is not a superuser, and does not bypass RLS.
--    If it were either, RLS and append-only history would both be inert.
-- --------------------------------------------------------------------------
DO $$
DECLARE r record;
BEGIN
    SELECT rolsuper, rolbypassrls INTO r FROM pg_roles WHERE rolname = 'taskforge_app';
    IF NOT FOUND THEN
        RAISE EXCEPTION 'taskforge_app role missing; RLS and append-only are inert';
    END IF;
    IF r.rolsuper OR r.rolbypassrls THEN
        RAISE EXCEPTION 'taskforge_app is superuser/bypassrls; every RLS policy is inert';
    END IF;
END $$;

-- --------------------------------------------------------------------------
-- 8. Exactly one initial status per workflow is enforceable.
-- --------------------------------------------------------------------------
DO $$
BEGIN
    IF NOT EXISTS (SELECT 1 FROM pg_indexes
                    WHERE schemaname='public' AND indexname='workflow_initial_uq') THEN
        RAISE EXCEPTION 'workflow_initial_uq missing; a workflow could have 0 or 2 entry points';
    END IF;
END $$;

\echo 'schema assertions: all passed'

-- --------------------------------------------------------------------------
-- 8. The pre-workspace seam returns a fixed projection (ADR-032, migration
--    0016).
--
--    `lookup_api_token` is SECURITY DEFINER: it reads through the RLS policy on
--    `api_token` because authentication happens before any workspace is known.
--    That is a deliberate hole in the ADR-020 backstop, and ADR-032 makes three
--    things non-optional. The other gates check TABLES, so a redefinition of
--    this function would pass every one of them.
--
--    Asserted here: the projection never includes the verifier hash, and the
--    search_path is pinned. A function whose RETURNS TABLE grew a
--    `verifier_hash` column would turn the seam into a credential-extraction
--    endpoint for any code holding EXECUTE.
-- --------------------------------------------------------------------------
DO $$
DECLARE
    definition text;
    config text[];
BEGIN
    SELECT pg_get_functiondef(p.oid), p.proconfig INTO definition, config
      FROM pg_proc p JOIN pg_namespace n ON n.oid = p.pronamespace
     WHERE n.nspname = 'public' AND p.proname = 'lookup_api_token';

    IF definition IS NULL THEN
        RAISE EXCEPTION 'lookup_api_token is missing; the pre-workspace seam does not exist';
    END IF;

    IF definition !~ 'SECURITY DEFINER' THEN
        RAISE EXCEPTION 'lookup_api_token is no longer SECURITY DEFINER; authentication cannot read the row';
    END IF;

    -- The projection. `verifier_hash` has its own door
    -- (lookup_api_token_verifier) precisely so it is never added to this one.
    IF definition ~* 'verifier_hash' THEN
        RAISE EXCEPTION 'lookup_api_token returns verifier_hash: the seam is now a credential-extraction endpoint';
    END IF;

    IF config IS NULL OR NOT (config @> ARRAY['search_path=public, pg_temp']) THEN
        RAISE EXCEPTION 'lookup_api_token has no pinned search_path (proconfig = %); a caller could shadow api_token', config;
    END IF;
END $$;

-- Nothing may hold EXECUTE on the seam except the application role. PUBLIC
-- EXECUTE on a SECURITY DEFINER function is the classic escalation.
DO $$
DECLARE granted text;
BEGIN
    -- DISTINCT: there are two functions, so the same grantee appears twice.
    SELECT string_agg(DISTINCT grantee, ',') INTO granted
      FROM information_schema.role_routine_grants
     WHERE routine_name IN ('lookup_api_token', 'lookup_api_token_verifier')
       AND grantee NOT IN ('taskforge_owner', current_user, 'tf', 'postgres');
    IF granted IS NOT NULL AND granted <> 'taskforge_app' THEN
        RAISE EXCEPTION 'unexpected EXECUTE on the pre-workspace seam: %', granted;
    END IF;
END $$;

-- --------------------------------------------------------------------------
-- 9. The membership seam is held to the same contract (migration 0019, C-002).
--
--    `workspace_membership` carries workspace_id and therefore a policy, while
--    the two reads that ESTABLISH a scope run before any workspace is known.
--    Same shape as the credential seam above, same three non-optional
--    properties — and checked here for the same reason: every other gate looks
--    at tables, so a redefinition of these functions would pass all of them.
-- --------------------------------------------------------------------------
DO $$
DECLARE
    fn text;
    definition text;
    config text[];
BEGIN
    FOREACH fn IN ARRAY ARRAY['is_workspace_member', 'workspace_ids_for_user'] LOOP
        SELECT pg_get_functiondef(p.oid), p.proconfig INTO definition, config
          FROM pg_proc p JOIN pg_namespace n ON n.oid = p.pronamespace
         WHERE n.nspname = 'public' AND p.proname = fn;

        IF definition IS NULL THEN
            RAISE EXCEPTION '% is missing; nobody can enter a workspace', fn;
        END IF;

        IF definition !~ 'SECURITY DEFINER' THEN
            RAISE EXCEPTION '% is no longer SECURITY DEFINER; the membership check answers false for everyone', fn;
        END IF;

        -- Filtered by the person, always. A seam that took only a workspace id
        -- would enumerate that workspace's members from outside it.
        IF definition !~ 'p_user' THEN
            RAISE EXCEPTION '% no longer filters by user; it can be pointed at somebody else', fn;
        END IF;

        -- A soft-deleted workspace is unreachable, not merely hidden
        -- (docs/32 §Deletion).
        IF definition !~ 'deleted_at' THEN
            RAISE EXCEPTION '% admits a soft-deleted workspace', fn;
        END IF;

        IF config IS NULL OR NOT (config @> ARRAY['search_path=public, pg_temp']) THEN
            RAISE EXCEPTION '% has no pinned search_path (proconfig = %); a caller could shadow workspace_membership', fn, config;
        END IF;
    END LOOP;
END $$;

DO $$
DECLARE granted text;
BEGIN
    SELECT string_agg(DISTINCT grantee, ',') INTO granted
      FROM information_schema.role_routine_grants
     WHERE routine_name IN ('is_workspace_member', 'workspace_ids_for_user')
       AND grantee NOT IN ('taskforge_owner', current_user, 'tf', 'postgres');
    IF granted IS NOT NULL AND granted <> 'taskforge_app' THEN
        RAISE EXCEPTION 'unexpected EXECUTE on the membership seam: %', granted;
    END IF;
END $$;

-- --------------------------------------------------------------------------
-- 10. Every mutable aggregate carries `version` (docs/24 §Optimistic
--     concurrency, docs/05 principle 5).
--
--     `workspace` and `team` were the two that did not, which meant the two
--     aggregates C-002 makes mutable were the two that could not express
--     `If-Match`. Listed rather than discovered, because "mutable aggregate" is
--     a design judgement and not something the catalogue knows.
-- --------------------------------------------------------------------------
DO $$
DECLARE missing text[];
BEGIN
    SELECT array_agg(t ORDER BY t) INTO missing
      FROM unnest(ARRAY['workspace','team','project','task','comment','role',
                        'saved_view']) t
     WHERE NOT EXISTS (SELECT 1 FROM pg_attribute a
                        WHERE a.attrelid = t::regclass
                          AND a.attname = 'version'
                          AND a.attnum > 0 AND NOT a.attisdropped);
    IF missing IS NOT NULL THEN
        RAISE EXCEPTION 'mutable aggregates without a version column: %', missing;
    END IF;
END $$;
