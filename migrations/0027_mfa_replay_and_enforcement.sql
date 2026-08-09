-- 0026 — What MFA needs that migration 0016 did not give it (C-001, docs/40 §MFA).
--
-- 0016 created `mfa_factor` and `recovery_code` and stopped there. Two columns
-- are missing, and each is missing a specific control rather than a feature.

-- ---------------------------------------------------------------------------
-- REPLAY REFUSAL (RFC 6238 §5.2)
-- ---------------------------------------------------------------------------
--
-- A TOTP code is valid for a whole 30-second step, so an attacker who observes
-- one — over a shoulder, through a phishing proxy, in a screen recording — can
-- present it themselves inside the same window. RFC 6238 §5.2 requires the
-- verifier to refuse a step it has already accepted.
--
-- `casual-task-identity::mfa` was built for this: `Totp::verify` returns the
-- matched TIME STEP rather than a bool, and its documentation says the caller
-- must reject a step it has already accepted. Nothing could, because there was
-- nowhere to remember one. This is that place.
--
-- Monotonic, not a set: storing only the highest accepted step also refuses
-- every EARLIER step, which is what closes the window on a code captured a few
-- seconds ago and replayed after the clock ticks on. A per-step set would be
-- larger, need sweeping, and permit exactly the replay it exists to stop.
ALTER TABLE mfa_factor ADD COLUMN last_step bigint;

COMMENT ON COLUMN mfa_factor.last_step IS
    'Highest TOTP time step accepted for this factor. RFC 6238 §5.2: a step at '
    'or below this is refused, so an observed code cannot be replayed inside '
    'its own window.';

-- ---------------------------------------------------------------------------
-- PER-WORKSPACE ENFORCEMENT
-- ---------------------------------------------------------------------------
--
-- docs/40 §Workspace-level SSO and MFA step-up: the browser session is
-- user-scoped — `user_account` is the only table without a workspace_id,
-- because a person spans workspaces — while MFA enforcement is per workspace.
-- A login therefore has no single policy to apply, and the policy is applied at
-- WORKSPACE RESOLUTION instead.
--
-- On `workspace` rather than in its `settings` jsonb: this is read on the path
-- of every workspace-scoped request, and a jsonb probe is neither indexable nor
-- typed. A security control that a typo in a settings key can silently switch
-- off is not a control.
--
-- Default false, and it stays false until an admin turns it on. docs/40 adds
-- the anti-lockout rule that makes turning it on safe — "the enforcing admin
-- must already have MFA enrolled, so nobody can lock themselves out while
-- locking others in" — and that is enforced in the handler, where the actor is
-- known.
ALTER TABLE workspace ADD COLUMN require_mfa boolean NOT NULL DEFAULT false;

COMMENT ON COLUMN workspace.require_mfa IS
    'docs/40 §Workspace-level SSO and MFA step-up. Enforced at workspace '
    'resolution, not at login: the session is user-scoped and this policy is '
    'per workspace.';

-- ---------------------------------------------------------------------------
-- The recovery-code read path
-- ---------------------------------------------------------------------------
--
-- 0016 indexed `recovery_code (user_id) WHERE used_at IS NULL`, which is what
-- redemption needs: every unused code for one person, then an Argon2 comparison
-- per row. Nothing further is needed here, and the index is named so that a
-- future reader looking for it does not add a second one.
--
-- No index on `mfa_factor`: it is keyed on `user_id` by a UNIQUE (user_id,
-- kind) constraint from 0016, and every lookup is by user.
