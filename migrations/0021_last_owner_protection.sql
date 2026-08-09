-- 0021 — Last-owner protection (D-054, docs/04 §Privilege escalation controls,
-- control 4).
--
-- docs/04: "The final grant carrying `workspace.owner` cannot be removed or
-- downgraded. Enforced as a database constraint check inside the transaction,
-- not just in application code."
--
-- WHAT THIS CLOSES
--
-- D-054 was "how does a workspace acquire its first grant". Answering it
-- creates a second question immediately: having acquired one, can it lose it?
-- A workspace whose last `workspace.owner` grant is revoked is in exactly the
-- state D-054 described — it exists, someone can see it, and nobody can
-- administer it — reached from the other direction. Bootstrapping without this
-- would fix the state on creation and leave it reachable one DELETE later.
--
-- WHY A TRIGGER AND NOT A CHECK CONSTRAINT
--
-- The fact being protected spans three tables: the assignment, the role it
-- names, and that role's permissions. A CHECK constraint sees one row of one
-- table. A trigger is the only mechanism PostgreSQL offers that can express it,
-- and it runs inside the caller's transaction, which is what docs/04 asks for.
--
-- WHY THIS DOES NOT ALSO GUARD `workspace` INSERT
--
-- The symmetric guarantee — "no workspace row exists without an owner grant" —
-- would be a DEFERRABLE INITIALLY DEFERRED constraint trigger firing at COMMIT.
-- It was considered and deliberately not taken here: nine call sites insert
-- workspaces directly (the EXPLAIN corpus's 100 workspaces, the 2M-row
-- reference corpus, the schema gate's own fixtures, four persistence tests),
-- and every one of them would have to mint an owner grant to keep working.
-- That changes the corpus the `explain-no-seq-scan` gate plans against, which
-- is a worse trade than it looks: the gate's value comes from the corpus being
-- stable. The creation direction is instead made impossible in the type system
-- — `workspace::insert` returns an `Unowned` that only `role::bootstrap`
-- can open — so a handler that creates a workspace without an owner does not
-- compile. Recorded so the stronger option stays visible.

CREATE FUNCTION protect_last_workspace_owner() RETURNS trigger
LANGUAGE plpgsql
AS $$
DECLARE
    remaining integer;
    surviving record;
BEGIN
    -- The row a BEFORE trigger must return differs by operation: OLD lets a
    -- DELETE proceed, NEW lets an UPDATE proceed. Returning OLD from a BEFORE
    -- UPDATE silently cancels the update instead of allowing it, which is a
    -- refusal nobody is told about.
    IF TG_OP = 'DELETE' THEN
        surviving := OLD;
    ELSE
        surviving := NEW;
    END IF;

    -- Only workspace-scope grants of a role carrying `workspace.owner` are
    -- protected. Every other assignment is ordinary and freely removable.
    IF OLD.scope_type <> 'WORKSPACE'
       OR NOT EXISTS (SELECT 1 FROM role_permission rp
                       WHERE rp.role_id = OLD.role_id
                         AND rp.permission = 'workspace.owner')
    THEN
        RETURN surviving;
    END IF;

    -- An UPDATE that leaves the row still carrying workspace.owner at
    -- workspace scope has transferred ownership, not removed it. docs/04
    -- forbids removal and downgrade; handing the same authority to a different
    -- principal is neither, and refusing it would make ownership untransferable.
    IF TG_OP = 'UPDATE'
       AND NEW.scope_type = 'WORKSPACE'
       AND NEW.workspace_id = OLD.workspace_id
       AND EXISTS (SELECT 1 FROM role_permission rp
                    WHERE rp.role_id = NEW.role_id
                      AND rp.permission = 'workspace.owner')
    THEN
        RETURN NEW;
    END IF;

    -- Otherwise this row is ceasing to be an owner grant. Count the ones that
    -- would remain, excluding it.
    SELECT count(*) INTO remaining
      FROM role_assignment ra
      JOIN role_permission rp ON rp.role_id = ra.role_id
     WHERE ra.workspace_id = OLD.workspace_id
       AND ra.scope_type = 'WORKSPACE'
       AND rp.permission = 'workspace.owner'
       AND ra.id <> OLD.id;

    IF remaining = 0 THEN
        RAISE EXCEPTION
            'workspace % would be left with no owner', OLD.workspace_id
            USING ERRCODE = 'restrict_violation',
                  HINT = 'TF-AZN-0005: grant workspace.owner to someone else first';
    END IF;

    RETURN surviving;
END;
$$;

COMMENT ON FUNCTION protect_last_workspace_owner() IS
    'docs/04 control 4. Refuses the removal or downgrade of the last '
    'WORKSPACE-scope grant carrying workspace.owner. A workspace with no owner '
    'is administrable by nobody and cannot recover on its own.';

-- BEFORE, not AFTER: the check has to refuse the statement rather than undo it.
-- DELETE covers the CASCADE from `role` too — dropping the Owner role deletes
-- its assignments, and this fires on each.
CREATE TRIGGER role_assignment_last_owner_del
    BEFORE DELETE ON role_assignment
    FOR EACH ROW EXECUTE FUNCTION protect_last_workspace_owner();

-- The "downgraded" half: moving the last owner assignment onto a different
-- role, or out of WORKSPACE scope.
CREATE TRIGGER role_assignment_last_owner_upd
    BEFORE UPDATE OF role_id, scope_type, scope_id, principal_id
    ON role_assignment
    FOR EACH ROW EXECUTE FUNCTION protect_last_workspace_owner();
