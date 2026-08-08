-- 0016 — Local authentication: credentials, sessions, MFA, invitations (C-001).
--
-- docs/40 §Where auth state lives. Everything here except `invitation` is keyed
-- on `user_account` and carries NO workspace_id, because a session and a
-- password belong to a PERSON, not to a tenant — `user_account` is already the
-- one table exempt from the tenancy backstop for exactly that reason.
--
-- That is the dangerous kind of exemption: these tables fall outside migration
-- 0010's catalogue loop by construction rather than by decision, so the reason
-- is recorded here AND in tests/schema/assertions.sql, where the gate that
-- would otherwise flag them lists them by name.

-- ---------------------------------------------------------------------------
-- SELECTOR / VERIFIER — the shape every credential in this file uses
-- ---------------------------------------------------------------------------
--
-- docs/40 §Credential lookup. A presented credential is `<selector>.<verifier>`:
--
--   selector  — non-secret, uniquely indexed. Finds the row in one index read.
--   verifier  — ~190 bits of randomness, stored only as a per-row salted hash.
--
-- Authentication is one indexed read followed by a constant-time comparison.
-- A database dump therefore contains no usable credential, without a
-- server-held pepper being load-bearing for every authentication (rejected in
-- ADR-032: lose it and every session dies; rotate it and they die unless a
-- versioning window exists).
--
-- Why not one hashed column with a UNIQUE index — the shape `api_token` had?
-- Because finding the row then requires hashing the presented secret with the
-- row's salt, which requires already knowing the row. It only works with an
-- unsalted hash, and an unsalted hash of a credential is a rainbow-table target
-- and leaks equality between rows.

CREATE TABLE user_credential (
    user_id       uuid PRIMARY KEY REFERENCES user_account(id) ON DELETE CASCADE,
    -- argon2id, PHC string: parameters travel with the hash, so raising the
    -- cost later does not invalidate existing passwords.
    --
    -- Argon2id and not the selector/verifier scheme above: a password is a
    -- LOW-ENTROPY secret chosen by a human, and a slow KDF is the only thing
    -- standing between a dump and an offline dictionary attack. The tokens
    -- elsewhere in this file are 190-bit random values, where a slow hash buys
    -- nothing and costs latency on every request.
    password_hash text NOT NULL,
    -- Forces re-authentication everywhere on password change: sessions created
    -- before this instant are refused (docs/40 §Local authentication).
    changed_at    timestamptz NOT NULL DEFAULT now(),
    -- Brute-force backoff (docs/40 §Acceptance gates: "without locking a
    -- legitimate user out permanently"). A counter and a time, not a boolean
    -- lock — a lock is a denial-of-service anyone can trigger by typing a
    -- stranger's email wrongly enough times.
    failed_attempts integer NOT NULL DEFAULT 0,
    locked_until  timestamptz
);

CREATE TABLE session (
    id              uuid PRIMARY KEY,
    user_id         uuid NOT NULL REFERENCES user_account(id) ON DELETE CASCADE,
    selector        text NOT NULL UNIQUE,
    verifier_hash   text NOT NULL,
    -- How this session was authenticated, and when MFA was last satisfied.
    -- docs/40 §Workspace-level SSO and MFA step-up: the session is user-scoped
    -- but MFA policy is per workspace, so "signed in" and "may enter this
    -- workspace" are two questions and the second needs these two columns.
    auth_method     text NOT NULL,
    mfa_satisfied_at timestamptz,
    created_at      timestamptz NOT NULL DEFAULT now(),
    last_seen_at    timestamptz NOT NULL DEFAULT now(),
    expires_at      timestamptz NOT NULL,
    -- Revocation is immediate, which is the entire reason docs/40 rejects JWTs.
    -- Nothing caches this row (ADR-032 withdrew the Redis read-through cache
    -- for the same reason: a cache reintroduces the staleness window the
    -- argument against JWTs rejects).
    revoked_at      timestamptz,
    ip_address      inet,
    user_agent      text
);

-- "Sign me out everywhere", and the session list a user is shown.
CREATE INDEX session_user_ix ON session (user_id, created_at DESC);
-- The expiry sweep. Partial, so it holds only sessions that still matter.
CREATE INDEX session_expiry_ix ON session (expires_at) WHERE revoked_at IS NULL;

CREATE TABLE mfa_factor (
    id           uuid PRIMARY KEY,
    user_id      uuid NOT NULL REFERENCES user_account(id) ON DELETE CASCADE,
    kind         text NOT NULL,          -- 'totp'
    -- The shared secret, base32. Stored recoverable BY NECESSITY: TOTP
    -- verification recomputes the code from it, so it cannot be hashed. It is
    -- therefore the one genuinely sensitive plaintext in the schema, and that
    -- is stated rather than left for a reader to notice.
    secret       text NOT NULL,
    -- NULL until the user proves they enrolled correctly by returning a code.
    -- An unconfirmed factor must never be treated as satisfying MFA: a user who
    -- lost the enrolment halfway would otherwise be locked out by a factor they
    -- do not have.
    confirmed_at timestamptz,
    created_at   timestamptz NOT NULL DEFAULT now(),
    UNIQUE (user_id, kind)
);

CREATE TABLE recovery_code (
    id         uuid PRIMARY KEY,
    user_id    uuid NOT NULL REFERENCES user_account(id) ON DELETE CASCADE,
    -- Hashed, like a password: a recovery code IS an authentication factor, and
    -- a dump of these is a dump of MFA bypasses.
    code_hash  text NOT NULL,
    -- Single use. Set on redemption rather than deleted, so "a recovery code
    -- was used" survives in the audit trail.
    used_at    timestamptz,
    created_at timestamptz NOT NULL DEFAULT now()
);
CREATE INDEX recovery_code_user_ix ON recovery_code (user_id) WHERE used_at IS NULL;

CREATE TABLE password_reset_token (
    id            uuid PRIMARY KEY,
    user_id       uuid NOT NULL REFERENCES user_account(id) ON DELETE CASCADE,
    selector      text NOT NULL UNIQUE,
    verifier_hash text NOT NULL,
    expires_at    timestamptz NOT NULL,
    used_at       timestamptz,
    created_at    timestamptz NOT NULL DEFAULT now()
);

-- ---------------------------------------------------------------------------
-- INVITATIONS — the one table here that IS tenant data
-- ---------------------------------------------------------------------------
CREATE TABLE invitation (
    id            uuid PRIMARY KEY,
    workspace_id  uuid NOT NULL REFERENCES workspace(id) ON DELETE CASCADE,
    email         citext NOT NULL,
    -- The role the invitee gets on acceptance. Nullable so an invitation can
    -- outlive a deleted role rather than blocking its deletion.
    role_id       uuid REFERENCES role(id) ON DELETE SET NULL,
    invited_by    uuid REFERENCES user_account(id),
    selector      text NOT NULL UNIQUE,
    verifier_hash text NOT NULL,
    expires_at    timestamptz NOT NULL,
    accepted_at   timestamptz,
    revoked_at    timestamptz,
    created_at    timestamptz NOT NULL DEFAULT now()
);

-- One live invitation per email per workspace. Partial, so a re-invite after
-- acceptance or revocation is allowed while a duplicate pending one is not.
CREATE UNIQUE INDEX invitation_pending_ix ON invitation (workspace_id, email)
    WHERE accepted_at IS NULL AND revoked_at IS NULL;

ALTER TABLE invitation ENABLE ROW LEVEL SECURITY;
ALTER TABLE invitation FORCE ROW LEVEL SECURITY;
CREATE POLICY invitation_tenant_isolation ON invitation
    USING (workspace_id = NULLIF(current_setting('taskforge.workspace_id', true), '')::uuid);

GRANT SELECT, INSERT, UPDATE ON invitation TO taskforge_app;
GRANT SELECT, INSERT, UPDATE, DELETE ON user_credential TO taskforge_app;
GRANT SELECT, INSERT, UPDATE, DELETE ON session TO taskforge_app;
GRANT SELECT, INSERT, UPDATE, DELETE ON mfa_factor TO taskforge_app;
GRANT SELECT, INSERT, UPDATE, DELETE ON recovery_code TO taskforge_app;
GRANT SELECT, INSERT, UPDATE, DELETE ON password_reset_token TO taskforge_app;

-- ---------------------------------------------------------------------------
-- api_token: token_hash -> selector + verifier (ADR-032, "migration required")
-- ---------------------------------------------------------------------------
--
-- The table is empty, so this is a rename rather than a data migration. Doing
-- it now is the whole point: after the first token is issued it is a breaking
-- change requiring every customer to reissue.
ALTER TABLE api_token DROP COLUMN token_hash;
ALTER TABLE api_token ADD COLUMN token_selector text NOT NULL;
ALTER TABLE api_token ADD COLUMN verifier_hash  text NOT NULL;
ALTER TABLE api_token ADD CONSTRAINT api_token_selector_key UNIQUE (token_selector);

-- ---------------------------------------------------------------------------
-- THE PRE-WORKSPACE SEAM (ADR-032 §The pre-workspace seam)
-- ---------------------------------------------------------------------------
--
-- `api_token` and `invitation` carry workspace_id and keep their policies.
-- Authentication happens BEFORE any workspace is known, so the request that
-- must read the credential row is exactly the one that cannot yet set
-- taskforge.workspace_id.
--
-- This function is the single door through the policy, and it is a FIXED
-- PROJECTION: identifying material only. It never returns verifier_hash, so it
-- cannot be used to extract a credential even by the code that is allowed to
-- call it. Callers verify by presenting the verifier and comparing hashes
-- themselves — see casual-task-identity.
--
-- THE COST, STATED (ADR-032): this is a deliberate hole in the ADR-020
-- backstop, and it is security-critical logic in SQL, outside the type system
-- and outside `unsafe_code = "forbid"`. A future edit widening its RETURNS
-- TABLE widens the hole silently. Three things are therefore NOT optional, and
-- all three are here:
--
--   1. search_path is pinned, so a caller cannot shadow `api_token` with their
--      own table and have a SECURITY DEFINER function read it.
--   2. Revoked and expired credentials return zero rows — asserted by a test.
--   3. The F-015 schema gate asserts this function's DEFINITION, not just its
--      existence. The gate checks tables today; a redefinition would pass.

CREATE FUNCTION lookup_api_token(p_selector text)
RETURNS TABLE (
    id             uuid,
    workspace_id   uuid,
    principal_type principal_type,
    principal_id   uuid
)
LANGUAGE sql
STABLE
SECURITY DEFINER
SET search_path = public, pg_temp
AS $$
    SELECT t.id, t.workspace_id, t.principal_type, t.principal_id
      FROM api_token t
     WHERE t.token_selector = p_selector
       AND t.revoked_at IS NULL
       AND (t.expires_at IS NULL OR t.expires_at > now());
$$;

REVOKE ALL ON FUNCTION lookup_api_token(text) FROM PUBLIC;
GRANT EXECUTE ON FUNCTION lookup_api_token(text) TO taskforge_app;

COMMENT ON FUNCTION lookup_api_token(text) IS
    'ADR-032 pre-workspace seam. Fixed projection: never returns verifier_hash. '
    'Widening RETURNS TABLE widens a deliberate hole in the ADR-020 backstop.';

-- The verifier itself is fetched separately, still through a fixed projection,
-- so that "find the row" and "read the secret" are two grants rather than one.
-- A future reader of the first function cannot conclude it is safe to add the
-- hash to it, because the hash already has its own door.
CREATE FUNCTION lookup_api_token_verifier(p_selector text)
RETURNS text
LANGUAGE sql
STABLE
SECURITY DEFINER
SET search_path = public, pg_temp
AS $$
    SELECT t.verifier_hash
      FROM api_token t
     WHERE t.token_selector = p_selector
       AND t.revoked_at IS NULL
       AND (t.expires_at IS NULL OR t.expires_at > now());
$$;

REVOKE ALL ON FUNCTION lookup_api_token_verifier(text) FROM PUBLIC;
GRANT EXECUTE ON FUNCTION lookup_api_token_verifier(text) TO taskforge_app;
