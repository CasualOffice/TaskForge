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
