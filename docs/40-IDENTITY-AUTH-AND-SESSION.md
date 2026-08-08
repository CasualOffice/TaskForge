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

*Revisited 2026-08-09 and kept.* The proposal was the standard modern shape:
the session row as a refresh token, short-lived JWTs as access tokens. It was
weighed on what it would buy **here** rather than in general.

What a JWT saves is the session lookup — one indexed primary-key read, ~0.1 ms
against the 150 ms p95 budget in [30](30-PERFORMANCE-AND-CAPACITY-TARGETS.md),
and signature verification is not free either. What it costs is the property
this section exists for: an admin revoking access, or a person being removed
from a workspace, keeps working until the access token expires.

JWTs earn that trade when the verifier **cannot reach the session store** —
several services, a separate auth service, verification at an edge. TaskForge is
one binary plus PostgreSQL ([48](48-DEPLOYMENT-PROFILES.md) Profile 1) and every
request already touches that database, so the saving is a fraction of a
millisecond and the cost is a real revocation window.

Machine credentials are the case where the trade does pay — they do not need
human-immediate revocation. That option stays open for the token surface in
[§Tokens](#tokens); it is not taken for browser sessions.

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
tf_pat_<16 bytes base62 selector><32 bytes base62 verifier>   personal access token
tf_sat_<16 bytes base62 selector><32 bytes base62 verifier>   service account token
```

- **Selector/verifier** (ADR-032). The selector is a non-secret lookup key with
  its own unique index; the verifier is the secret, stored as a per-row salted
  hash and compared in constant time. Lookup is one indexed read on the
  selector, which is what [21](21-API-LIMITS-AND-QUOTAS.md) §Query limits
  budgets, and the plaintext is displayed once and is unrecoverable. A database
  dump is not a credential dump.
- The token is longer than the earlier `tf_pat_<32 bytes>` form, deliberately.
  That is the price of having **no server-held pepper**: nothing outside the
  database is load-bearing for authentication, so there is no key to lose, no
  key to rotate, and no key-custody procedure to get wrong at 3 a.m.
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

## Mechanism (ADR-032, **Accepted**)

Everything above is the auth *protocol* and is unchanged. This is the layer
beneath it: how a presented credential is found, where auth state lives, and how
a request that has no workspace yet reaches a row the tenancy backstop hides.

It exists because the design record and the **already-`Gated`** schema
contradicted each other in four places. Accepted with amendments — two of the
proposals were rejected in favour of better ones, and both rejections removed a
cost rather than adding one.

### Credential lookup: selector/verifier, no pepper

`api_token` and `session` store a **selector** (non-secret, uniquely indexed)
and a **hash of the verifier** (per-row salt). Authentication is one indexed
read on the selector followed by a constant-time comparison of the verifier
hash — which is what [21](21-API-LIMITS-AND-QUOTAS.md) budgets, and what the
`UNIQUE` index on the old `token_hash` column was already shaped for.

A keyed HMAC under a server-held pepper was proposed and **rejected**. It would
have made a secret outside the database load-bearing for every authentication:
lose it and every session and token dies, rotate it and they die unless a
versioning window exists, which forces `hash_key_id` on two tables and a key
custody procedure into the runbooks. Selector/verifier buys the same property —
a database dump contains no usable credential, because the verifier hash is
salted per row and the tokens are ~190 bits — for a longer token and no key.

Argon2id stays exactly where it was, on passwords, where a low-entropy secret
genuinely needs a slow KDF.

**Migration required:** `api_token.token_hash text NOT NULL UNIQUE` becomes
`token_selector` + `verifier_hash`. Cheap now; the table is empty.

### `TF_SECRET_KEY` is not a cookie signature

The session cookie stays opaque and unsigned — a signature over a random value
proves nothing the value does not already prove. The key is used for the CSRF
binding. [48](48-DEPLOYMENT-PROFILES.md) describing it as "session/cookie
signing" is what changes.

### Sessions and tokens are never cached

The optional Redis read-through cache is **withdrawn**. "Revocation is
immediate" is the entire stated reason this document rejects JWTs, and a cache
reintroduces exactly the staleness window that argument rejects. The lookup is
one indexed read — already cheaper than verifying a signature, which is the same
argument, applied consistently.

### A plugin installation is an auth actor, not an RBAC principal

`principal_type` is **not** extended, and the `ENUM ('USER','TEAM',
'SERVICE_ACCOUNT')` in migration 0001 stands.

Extending it was proposed and **rejected**. A plugin installation authenticates,
but it is not something a role is assigned to: its authority is the scoped token
issued to it, bounded by the installing actor's permissions
([34](34-PLUGIN-AND-EXTENSION-ARCHITECTURE.md)). Making it a `principal_type`
would have put it in the resolver's principal set and invited grants to be
assigned to installations directly — a second, parallel authority model reaching
the same resources. Keeping the enum closed keeps [04](04-RBAC-AND-AUTHORIZATION.md)
the only answer to "who may do what", and avoids a schema change entirely.

### The pre-workspace seam: a tightly scoped `SECURITY DEFINER`

`api_token` and `invitation` both carry `workspace_id` and stay RLS-covered.
Authentication happens before any workspace is known, so the request that must
read the credential row is exactly the one that cannot yet set
`taskforge.workspace_id`.

A `SECURITY DEFINER` function returns a **fixed projection** — identifying
material only, never the stored verifier hash — with `EXECUTE` granted to
`taskforge_app` and `search_path` pinned. The table keeps its policy; the single
door through it is a fixed shape rather than a `SELECT`.

**The cost, and the conditions.** This is a deliberate hole in the ADR-020
backstop, and it is security-critical logic in SQL, outside the type system and
outside `unsafe_code = "forbid"`. A future edit widening its `RETURNS TABLE`
widens the hole silently. Three things are therefore not optional: the pinned
`search_path`, a test asserting it returns zero rows for a revoked or expired
credential, and an extension of the F-015 schema gate to assert the function's
**definition** — the gate checks tables today, so a redefinition would pass.

### Workspace-level SSO and MFA step-up

The browser session is user-scoped. `user_account` is the only table without
`workspace_id` — a person spans workspaces — while `enforce_sso`, MFA
enforcement and `allowed_domains` are per-workspace, so a login has no single
policy to apply.

Per-workspace policy is enforced at **workspace resolution**, not at login. The
session records how it was authenticated (`auth_method`, `mfa_satisfied_at`);
entering a workspace that demands more than the session carries triggers a
step-up. The cost is that "signed in" and "may enter this workspace" are two
questions, and every workspace-scoped entry point must ask the second.

### Where auth state lives

One migration adds `session`, `user_credential`, `mfa_factor`, `recovery_code`,
`password_reset_token` and `invitation`. All except `invitation` are keyed on
`user_account`, carry no `workspace_id`, and so fall outside migration 0010's
catalogue loop **by construction** — the dangerous kind of exemption. It is
therefore written into 0010's existing exemption block, in the same style as
`outbox_event` and `user_account`, with the reason recorded: a session and a
password belong to a person.
