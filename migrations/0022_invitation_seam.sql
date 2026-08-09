-- 0022 — The invitation acceptance seam (C-001, docs/40 §Invitations).
--
-- ADR-032 §The pre-workspace seam named this table when it was written:
--
--   "`api_token` and `invitation` both carry workspace_id and stay RLS-covered.
--    Authentication happens before any workspace is known, so the request that
--    must read the credential row is exactly the one that cannot yet set
--    `taskforge.workspace_id`."
--
-- Accepting an invitation is the strongest form of that problem. The caller is
-- not merely outside the workspace — they may have no account at all, which is
-- the entire point of inviting by email. There is no `WorkspaceScope` to apply,
-- and there cannot be one until the invitation itself says which workspace.
--
-- This is the same failure C-002 shipped and had to fix in migration 0020: read
-- unscoped as `taskforge_app`, the policy hides every row, the lookup returns
-- nothing, and EVERY acceptance fails. It would have passed every test, because
-- the harness connects as the database owner and RLS is inert for a superuser
-- (migration 0012). Writing the seam now is cheaper than discovering it twice.
--
-- The treatment is the one ADR-032 fixed and migrations 0016 and 0020 both
-- follow: SECURITY DEFINER, a pinned `search_path`, a fixed and minimal
-- projection, EXECUTE granted to `taskforge_app` alone.
--
-- THE COST, STATED. This is a third deliberate hole in the ADR-020 backstop,
-- and it is bounded by what the functions can return. Both are keyed on the
-- SELECTOR — 96 bits of unguessable randomness that only the holder of the
-- emailed link has — never on a workspace id and never on an email address.
-- Neither can therefore be used to enumerate a workspace's invitations, or to
-- ask whether a given address was invited. `lookup_invitation` returns no
-- verifier; the verifier has its own door, so a future reader cannot conclude
-- it would be convenient to widen the first one.

-- The invitation behind a presented selector, if it is still live.
--
-- `email` is in the projection because docs/40 §Invitations requires the
-- invitation to be "tied to the address": the acceptance path compares it
-- against the account being used, and refusing that comparison is what stops an
-- invitation being a bearer token for whoever reads the mailbox.
CREATE FUNCTION lookup_invitation(p_selector text)
RETURNS TABLE (
    id           uuid,
    workspace_id uuid,
    email        citext,
    role_id      uuid
)
LANGUAGE sql
STABLE
SECURITY DEFINER
SET search_path = public, pg_temp
AS $$
    SELECT i.id, i.workspace_id, i.email, i.role_id
      FROM invitation i
      JOIN workspace w ON w.id = i.workspace_id
     WHERE i.selector = p_selector
       AND i.accepted_at IS NULL
       AND i.revoked_at IS NULL
       AND i.expires_at > now()
       -- A workspace in its 30-day deletion grace window (docs/32 §Deletion) is
       -- unreachable rather than merely hidden, exactly as migration 0020's
       -- membership seam treats it. Joining a workspace that is being deleted
       -- is not a thing anyone should be able to do.
       AND w.deleted_at IS NULL;
$$;

REVOKE ALL ON FUNCTION lookup_invitation(text) FROM PUBLIC;
GRANT EXECUTE ON FUNCTION lookup_invitation(text) TO taskforge_app;

COMMENT ON FUNCTION lookup_invitation(text) IS
    'ADR-032 pre-workspace seam. Fixed projection: never returns verifier_hash. '
    'Keyed on the selector only, so it cannot enumerate a workspace''s '
    'invitations. Widening RETURNS TABLE widens a deliberate hole in the '
    'ADR-020 backstop.';

-- The verifier, behind its own grant. See migration 0016: "find the row" and
-- "read the secret" are two doors so that nobody concludes the first one should
-- carry the second.
CREATE FUNCTION lookup_invitation_verifier(p_selector text)
RETURNS text
LANGUAGE sql
STABLE
SECURITY DEFINER
SET search_path = public, pg_temp
AS $$
    SELECT i.verifier_hash
      FROM invitation i
      JOIN workspace w ON w.id = i.workspace_id
     WHERE i.selector = p_selector
       AND i.accepted_at IS NULL
       AND i.revoked_at IS NULL
       AND i.expires_at > now()
       AND w.deleted_at IS NULL;
$$;

REVOKE ALL ON FUNCTION lookup_invitation_verifier(text) FROM PUBLIC;
GRANT EXECUTE ON FUNCTION lookup_invitation_verifier(text) TO taskforge_app;

COMMENT ON FUNCTION lookup_invitation_verifier(text) IS
    'ADR-032 pre-workspace seam. Returns one hash for one selector. Separate '
    'from lookup_invitation so that finding a row and reading its secret are '
    'two grants rather than one.';

-- Burn an invitation, reporting whether THIS call was the one that burned it.
--
-- `accepted_at IS NULL` is in the WHERE clause, not in a preceding SELECT. That
-- is what makes single use a property of the database rather than of the order
-- two requests happen to arrive in: two concurrent acceptances both find a live
-- invitation, both reach here, and exactly one updates a row. Reading first and
-- updating second is the same code with a race in it.
--
-- It is SECURITY DEFINER for the same reason the lookup is — the acceptor has
-- no workspace scope — and it is a FUNCTION rather than an UPDATE in the
-- application so that the predicate cannot be separated from the write by a
-- caller who did not read this comment.
CREATE FUNCTION consume_invitation(p_id uuid)
RETURNS boolean
LANGUAGE sql
VOLATILE
SECURITY DEFINER
SET search_path = public, pg_temp
AS $$
    WITH burned AS (
        UPDATE invitation
           SET accepted_at = now()
         WHERE id = p_id
           AND accepted_at IS NULL
           AND revoked_at IS NULL
           AND expires_at > now()
        RETURNING 1)
    SELECT EXISTS (SELECT 1 FROM burned);
$$;

REVOKE ALL ON FUNCTION consume_invitation(uuid) FROM PUBLIC;
GRANT EXECUTE ON FUNCTION consume_invitation(uuid) TO taskforge_app;

COMMENT ON FUNCTION consume_invitation(uuid) IS
    'ADR-032 seam. Single use as a WHERE clause: returns true only for the '
    'call that burned the row, so concurrent acceptances resolve to one winner.';
