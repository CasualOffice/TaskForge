# 40 — Identity, Authentication & Session

The old drafts listed "OIDC/SAML readiness, optional local auth, MFA" as a
baseline and specified none of it. This is the specification.

Authentication answers *who is this*. Authorization
([04](04-RBAC-AND-AUTHORIZATION.md)) answers *what may they do*. They are
separate systems and never merged.

## Actors

| Actor | Credential | Lifetime |
| --- | --- | --- |
| **User (browser)** | session cookie | 14 d idle / 30 d absolute |
| **User (API)** | personal access token | user-set, max 1 y |
| **Service account** | API token | admin-set, max 1 y |
| **Plugin installation** | scoped token | 1 h, auto-refreshed |
| **Internal worker** | mTLS or shared secret | deployment-scoped |

Every one resolves to an `AuthContext { actor_id, actor_type, workspace_id }`,
the sole source of a `WorkspaceScope` ([32](32-TENANCY-AND-ISOLATION.md)).

## Browser sessions — cookies, not JWTs

```
Set-Cookie: tf_session=<opaque-256-bit>; HttpOnly; Secure; SameSite=Lax; Path=/
```

**Opaque, server-side sessions. Deliberately not JWTs.**

A JWT cannot be revoked before it expires. In a product whose entire premise is
explicit authority — where an admin revoking access expects it to *be* revoked —
a 15-minute window in which a removed user still has a valid token is a
correctness failure, not a performance trade. The usual mitigation (short expiry
+ refresh + a revocation list) reconstructs server-side sessions with more moving
parts and worse failure modes.

The session store is PostgreSQL, with Redis as an optional read-through cache. A
session lookup is one indexed primary-key read — cheaper than verifying a
signature — so the "stateless is faster" argument does not survive measurement
here.

- Rotated on privilege change and on login.
- **Revocation is immediate**: delete the row. Admin-visible session list, with
  "sign out everywhere."
- `SameSite=Lax` plus a double-submit CSRF token on every unsafe method.
- Session records carry IP and user agent for the audit trail; a change in either
  is surfaced to the user, not silently accepted.

## Local authentication

Optional, and off by default when SSO is configured.

- **Argon2id** password hashing (64 MB memory, t=3, p=4), parameters stored per
  hash so they can be raised without invalidating existing passwords.
- No composition rules beyond a 12-character minimum. Rules produce `Password1!`;
  length and a breach check produce better passwords.
- Breached-password check against a k-anonymity range API at set time, with a
  local fallback list when the deployment is air-gapped.
- Rate limited per account **and** per IP, with exponential lockout
  ([21](21-API-LIMITS-AND-QUOTAS.md)).
- Reset tokens: single-use, 1 h, hashed at rest, invalidated by password change.
- **Login responses are constant-shape and constant-ish time** whether or not the
  account exists. Account enumeration through the login endpoint is the most
  commonly shipped auth bug.

## SSO

**OIDC is first-class; SAML follows.** OIDC because it is what modern IdPs
default to and what a self-hoster can configure without a consultant; SAML
because enterprise procurement still asks for it.

Per workspace:

```
issuer, client_id, client_secret (encrypted at rest),
scopes, claim mapping { email, name, groups },
jit_provisioning: bool, allowed_domains: [...], enforce_sso: bool
```

- Authorization Code + PKCE. Implicit flow is not supported.
- `state` and `nonce` verified; ID token signature verified against cached JWKS
  with bounded refresh.
- **JIT provisioning** creates a workspace membership on first login, with a
  configurable default role — never an elevated one.
- **Group→role mapping** is optional and, when enabled, **authoritative**: groups
  removed at the IdP remove the corresponding grants at next login. Half-syncing
  group membership produces permissions no one can explain the origin of.
- `enforce_sso` disables local login for the workspace, with a documented
  break-glass owner path that requires MFA and writes a prominent audit event —
  because an IdP misconfiguration must not permanently lock an owner out of their
  own workspace.

## MFA

- **TOTP** (RFC 6238) baseline, with 10 single-use recovery codes shown once.
- **WebAuthn / passkeys** as the preferred second factor and, later, as a primary
  factor — phishing-resistant in a way TOTP is not.
- Enforceable per workspace; the enforcing admin must already have MFA enrolled,
  so nobody can lock themselves out while locking others in.
- Re-authentication (not merely a valid session) is required for: changing
  password, managing MFA, creating API tokens, transferring ownership, and
  deleting a workspace.

## Tokens

```
tf_pat_<32 bytes base62>          personal access token
tf_sat_<32 bytes base62>          service account token
```

- Stored as **argon2id hashes**; the plaintext is displayed once and is
  unrecoverable. A database dump is not a credential dump.
- The prefix is deliberate: it makes secret-scanning tools (and our own
  pre-commit hook) able to detect a leaked token in a repository.
- Scoped to one workspace, with an optional permission subset **not exceeding the
  owner's** ([04](04-RBAC-AND-AUTHORIZATION.md)).
- `last_used_at` recorded, so unused tokens can be found and revoked.
- Revocation is immediate; expiry is enforced at verification.

## Invitations

- Invite by email, single-use, 7-day expiry, tied to the address.
- **The response is identical whether or not the address has an account** — only
  the delivered email differs. This is the counterpart to the login-enumeration
  rule ([32](32-TENANCY-AND-ISOLATION.md)).
- Accepting creates a workspace membership with the invited role, and nothing else.
- Domain-restricted open invite links are supported and rate limited.

## Plugin tokens

Issued per installation, per workspace, 1 h, auto-refreshed. Carry only the
consented scopes, intersected with the installing admin's permissions at install
time ([34](34-PLUGIN-AND-EXTENSION-ARCHITECTURE.md)). Revoked instantly on
uninstall, disable, or consent change — not on next refresh.

## What is audited

Every one of these writes an `audit_event` ([25](25-EVENTS-OUTBOX-AND-AUDIT.md)):

`auth.login` · `auth.login.failed` · `auth.logout` · `auth.session.revoked` ·
`auth.mfa.enrolled` · `auth.mfa.removed` · `auth.password.changed` ·
`auth.sso.configured` · `token.created` · `token.revoked` · `user.invited` ·
`user.deactivated` · `permission.denied`

Failed logins are audited with IP and user agent. A burst of them is the clearest
available signal of credential stuffing, and it is invisible if only successes are
recorded.

## Acceptance gates

- **Revocation test** — a revoked session is rejected on the next request; an SSE
  stream held by that session closes.
- **Enumeration test** — login, reset, and invite responses are indistinguishable
  for existing and non-existing accounts, in body, status, and timing envelope.
- **CSRF test** — every unsafe method without a valid token is rejected.
- **Token-hash test** — a database dump contains no usable credential.
- **SSO test** — against a real OIDC provider in CI (Keycloak container): claim
  mapping, JIT provisioning, group→role sync including *removal*, and
  signature-verification failure paths.
- **Lockout test** — brute force triggers exponential backoff without locking a
  legitimate user out permanently.
- **Break-glass test** — an owner locked out by a broken IdP can recover through
  the documented path, and the recovery is audited.

## Mechanism — **Proposed, not Accepted** (D-032 / ADR-032)

Everything above is settled and is not reopened here. What follows is the layer
*beneath* it: how a presented credential is found, where auth state lives, and
how a request that has no workspace yet reaches a row the tenancy backstop is
designed to hide.

It is written down because the design record and the **already-`Gated`** schema
contradict each other in four places, and two of the resolutions change that
schema — which is cheapest now, before the tables hold data.

**Status: `Proposed`.** A human accepts this. Until then D-032 stays `Blocked`,
and nothing below has been implemented.

### The four contradictions, and what is proposed

**1. A salted hash cannot be looked up.** `api_token.token_hash` is
`text NOT NULL UNIQUE` (migration 0008) and [21](21-API-LIMITS-AND-QUOTAS.md)
budgets authentication at "one indexed read". Both require a *deterministic*
digest. This document says tokens are "hashed at rest" without naming an
algorithm, two lines below specifying Argon2id for passwords — so an
implementer reaching for the nearest password hasher produces a token no query
can find.

*Proposed:* `token_hash` holds `HMAC-SHA256(pepper, token)` under a server-held
pepper. Argon2id exists to make a *low-entropy* secret expensive to guess; a
32-byte token carries ~190 bits of entropy, so the KDF buys nothing and costs
64 MB per verification **on attacker-controlled input**. The gate "a database
dump contains no usable credential" is satisfied more strongly, because the
pepper is not in the database.

*Cost:* the pepper becomes a credential-invalidating key — lose it and every
session and token dies. That forces `hash_key_id smallint` on both tables and a
rotation window that tries the current key then one predecessor, plus key
custody in a runbook that does not exist today.

*Judgement call:* a `selector`/`verifier` split avoids the pepper entirely, at
the cost of a longer token than this document specifies. Both satisfy every
gate. A reviewer who weights key custody above wire-format stability should
choose the split.

**2. `TF_SECRET_KEY` has no stated job.**
[48](48-DEPLOYMENT-PROFILES.md) calls it "session/cookie signing" and this
document specifies a plain opaque cookie, which has nothing to sign.

*Proposed:* the cookie stays opaque and unsigned — a signature over a random
value proves nothing that the value does not already prove — and the key is
what the HMAC above is keyed with, plus the CSRF binding. docs/48's description
is what changes.

**3. The Redis cache can outlive a revocation.** "Revocation is immediate:
delete the row" is the *entire* stated reason this document rejects JWTs, and a
read-through cache reintroduces exactly that staleness window.

*Proposed:* sessions and tokens are never cached. The lookup is one indexed
read on a primary-key-shaped index, which is already cheaper than verifying a
signature — the argument this document makes against JWTs applies to the cache
too. The optional-cache sentence is withdrawn.

**4. A plugin token has no principal.** `principal_type` is
`ENUM ('USER','TEAM','SERVICE_ACCOUNT')` (migration 0001) and this document
specifies a per-installation plugin token.

*Proposed:* extend the enum. An enum change to a `Gated` schema is cheap while
the table is empty and expensive afterwards, which is the argument for settling
this now rather than at C-017.

### Where auth state lives

No session, credential, MFA-factor, recovery-code, reset-token, invitation or
SSO-connection table exists — migrations 0001–0012 are `Gated` and define none
of them, and [05](05-API-SPEC.md) lists no auth endpoint.

*Proposed:* one migration adds them. All except `invitation` are keyed on
`user_account`, carry no `workspace_id`, and therefore fall outside migration
0010's catalogue loop **by construction** — which is the dangerous kind of
exemption, so it is proposed as a *written* one in 0010's existing exemption
block, in the same style as `outbox_event` and `user_account`.

### The pre-workspace seam

`api_token` and `invitation` both carry `workspace_id`, so both are RLS-covered.
But authentication happens *before* any workspace is known: the request that
must read the token row is exactly the request that cannot yet set
`taskforge.workspace_id`.

*Proposed:* a `SECURITY DEFINER` function returning a fixed projection —
identifying material only, never the digest — with `EXECUTE` granted to
`taskforge_app` and `search_path` pinned. The table keeps its policy; the one
door through it is a fixed shape rather than a `SELECT`.

*Cost, plainly:* a `SECURITY DEFINER` function is a deliberate hole in the
ADR-020 backstop, and it is security-critical logic living in SQL — outside the
type system and outside `unsafe_code = "forbid"`. A future edit widening its
`RETURNS TABLE` widens the hole silently, so the F-015 schema gate would need to
assert the function's definition, not only the tables it currently checks.

### Which workspace's policy governs a login

`user_account` is the only table without `workspace_id` — a person spans
workspaces — while `enforce_sso`, MFA enforcement and `allowed_domains` are
per-workspace. So a login has no single policy to apply.

*Proposed:* the browser session is user-scoped, and per-workspace policy is
enforced at **workspace resolution** rather than at login. A session records
how it was authenticated (`auth_method`, `mfa_satisfied_at`); entering a
workspace that demands more than the session carries triggers a step-up. The
cost is that "signed in" and "may enter this workspace" become two questions,
and every workspace-scoped entry point has to ask the second.
