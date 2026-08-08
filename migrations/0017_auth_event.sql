-- 0017 — The authentication audit trail (docs/40 §What is audited).
--
-- WHY NOT audit_event
--
-- `audit_event.workspace_id` is NOT NULL, and authentication happens before any
-- workspace is known — a failed login for an address that matches no account
-- has no workspace at all. Forcing these into that table would mean inventing a
-- workspace id for the events an incident responder most needs.
--
-- So they live beside `session` and `user_credential`: keyed on a person, not a
-- tenant, and exempt from the tenancy backstop for the same stated reason.

CREATE TABLE auth_event (
    id          uuid PRIMARY KEY,
    -- NULL when the attempt named an address with no account. That row is the
    -- point: docs/40 "A burst of them is the clearest signal of an attack in
    -- progress", and an attacker guessing addresses produces exactly these.
    user_id     uuid REFERENCES user_account(id) ON DELETE SET NULL,
    -- What was typed, lowercased. Retained because an attack is visible in the
    -- pattern of addresses tried, which is lost if only matches are recorded.
    email       citext,
    -- login.succeeded | login.failed | login.locked | logout | session.revoked
    event_type  text NOT NULL,
    ip_address  inet,
    user_agent  text,
    occurred_at timestamptz NOT NULL DEFAULT now()
);

-- "Is this account under attack?" and "what happened to this person?"
CREATE INDEX auth_event_user_ix  ON auth_event (user_id, occurred_at DESC);
-- "Is this address being sprayed?" — the unknown-account case, where user_id is
-- NULL and the index above cannot answer.
CREATE INDEX auth_event_email_ix ON auth_event (email, occurred_at DESC);
-- "Is one source responsible?" — the question a credential-stuffing incident
-- opens with.
CREATE INDEX auth_event_ip_ix    ON auth_event (ip_address, occurred_at DESC);

-- Append-only, enforced by the absence of the privilege rather than by
-- application discipline — the same mechanism as audit_event (migration 0012).
-- An authentication trail an attacker can edit after using a stolen session is
-- not a trail.
GRANT SELECT, INSERT ON auth_event TO taskforge_app;
