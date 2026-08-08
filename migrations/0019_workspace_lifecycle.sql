-- 0019 — Workspace lifecycle: optimistic concurrency, and the membership seam
-- (C-002). See docs/24 §Optimistic concurrency, docs/32 §The user_account
-- exception, ADR-032 §The pre-workspace seam.

-- ---------------------------------------------------------------------------
-- 1. `version` on the two mutable aggregates that were missing it
-- ---------------------------------------------------------------------------
--
-- docs/24 opens with "every mutable aggregate carries `version bigint`,
-- incremented on every write", exposed as an `ETag` and required as `If-Match`;
-- docs/05 principle 5 says the same. `project`, `task`, `comment`, `role` and
-- `saved_view` all carry it. `workspace` and `team` did not, so the two
-- aggregates C-002 makes mutable were the two that could not express the
-- contract the API spec promises.
--
-- Added now rather than when a rename endpoint first needs it, because the
-- direction of that change is one-way: shipping `PATCH /workspaces/{id}`
-- without `If-Match` and adding the requirement later is a BREAKING API change
-- (a newly required request header, and a 428 where a client used to get 200).
ALTER TABLE workspace ADD COLUMN version bigint NOT NULL DEFAULT 1;
ALTER TABLE team      ADD COLUMN version bigint NOT NULL DEFAULT 1;

-- ---------------------------------------------------------------------------
-- 2. THE MEMBERSHIP SEAM (ADR-032 §The pre-workspace seam, docs/32)
-- ---------------------------------------------------------------------------
--
-- WHY THIS EXISTS, AND WHAT WAS BROKEN WITHOUT IT
--
-- `workspace_membership` carries `workspace_id`, so migration 0010's catalogue
-- loop gave it a policy: a row is visible only when
-- `workspace_id = current_setting('taskforge.workspace_id')`. That is correct
-- for every ordinary read.
--
-- It is fatal for the two reads that ESTABLISH the scope, which necessarily run
-- before any workspace has been set:
--
--   * "is this actor a member of the workspace they are claiming?" — the check
--     that mints the `AuthContext` (`docs/05` §Authentication: "validated
--     against membership on every request"). Run unscoped as `taskforge_app`,
--     the policy hides every row, `EXISTS` returns false, and **no one can ever
--     enter any workspace**. It passed every test because the test harness
--     connects as a superuser, for whom RLS is inert (migration 0012).
--
--   * "which workspaces does this person belong to?" — `GET /api/v1/workspaces`
--     is inherently cross-tenant. There is no single workspace to scope it to;
--     that is the question being asked.
--
-- This is the same shape as the credential seam in migration 0016 and it gets
-- the same treatment: SECURITY DEFINER, a pinned `search_path`, a fixed and
-- minimal projection, EXECUTE granted to `taskforge_app` alone, and an
-- assertion in the F-015 schema gate over the DEFINITION rather than the
-- existence.
--
-- THE COST, STATED. This is a second deliberate hole in the ADR-020 backstop.
-- It is bounded by what the functions can return: a person's own membership,
-- and nothing else. Neither takes a workspace id it does not also filter a user
-- id by, so neither can be used to enumerate a workspace's members, and neither
-- returns any column of any tenant table beyond the workspace ids the caller
-- themselves belongs to. Both exclude soft-deleted workspaces, so a workspace
-- in its 30-day grace window (docs/32 §Deletion) is unreachable rather than
-- merely hidden.

CREATE FUNCTION is_workspace_member(p_user uuid, p_workspace uuid)
RETURNS boolean
LANGUAGE sql
STABLE
SECURITY DEFINER
SET search_path = public, pg_temp
AS $$
    SELECT EXISTS (
        SELECT 1
          FROM workspace_membership m
          JOIN workspace w ON w.id = m.workspace_id
         WHERE m.user_id = p_user
           AND m.workspace_id = p_workspace
           AND w.deleted_at IS NULL);
$$;

REVOKE ALL ON FUNCTION is_workspace_member(uuid, uuid) FROM PUBLIC;
GRANT EXECUTE ON FUNCTION is_workspace_member(uuid, uuid) TO taskforge_app;

COMMENT ON FUNCTION is_workspace_member(uuid, uuid) IS
    'ADR-032 seam. Answers one question about ONE named person and ONE named '
    'workspace, as a boolean. Widening the return type widens a deliberate hole '
    'in the ADR-020 backstop.';

-- The workspace ids a person belongs to. Ids only: the workspace rows
-- themselves are read from `workspace`, which is exempt from RLS because row
-- identity *is* the tenant (migration 0010), so the caller needs nothing more
-- from behind the policy than the set of ids.
CREATE FUNCTION workspace_ids_for_user(p_user uuid)
RETURNS SETOF uuid
LANGUAGE sql
STABLE
SECURITY DEFINER
SET search_path = public, pg_temp
AS $$
    SELECT m.workspace_id
      FROM workspace_membership m
      JOIN workspace w ON w.id = m.workspace_id
     WHERE m.user_id = p_user
       AND w.deleted_at IS NULL;
$$;

REVOKE ALL ON FUNCTION workspace_ids_for_user(uuid) FROM PUBLIC;
GRANT EXECUTE ON FUNCTION workspace_ids_for_user(uuid) TO taskforge_app;

COMMENT ON FUNCTION workspace_ids_for_user(uuid) IS
    'ADR-032 seam. Returns the workspace ids ONE named person belongs to, and '
    'nothing else. It is filtered by user, never by workspace, so it cannot '
    'enumerate a workspace''s members.';
