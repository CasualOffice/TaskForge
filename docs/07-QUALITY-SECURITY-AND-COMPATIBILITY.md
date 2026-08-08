# 07 — Quality, Security & Compatibility

The non-negotiables. Where a decision in another doc conflicts with something
here, this document wins.

## Engineering priorities (ordered)

When two goals conflict, the earlier one wins:

1. **Correctness & authority** — never grant access that was not granted; never
   lose a change that was accepted.
2. **Tenant isolation** — no data crosses a workspace boundary, ever.
3. **Traceability** — every material change is attributable and immutable.
4. **Security & resource bounds** — every input bounded; every external call
   timed; no customer code in-process.
5. **Data durability** — backups verified by restore, not by existence.
6. **Performance** — the gates in [30](30-PERFORMANCE-AND-CAPACITY-TARGETS.md).
7. **API stability** — the public surface is narrower than internals and
   versioned.
8. **UX** — fast, keyboard-first, progressively disclosed.
9. **Maintainability.**

## Threat model

Assets, ranked: workspace data · credentials and tokens · the audit trail ·
attachments · plugin secrets.

| Threat | Control |
| --- | --- |
| Cross-tenant access | `WorkspaceScope` type + RLS backstop; cross-tenant property test over every endpoint ([32](32-TENANCY-AND-ISOLATION.md)). **One deliberate exception:** the pre-workspace credential lookup runs through a fixed `SECURITY DEFINER` projection, because authentication precedes knowing the workspace (ADR-032). Its definition is gate-asserted, not assumed |
| Privilege escalation | Grant and scope ceilings; self-elevation block; last-owner protection; escalation test suite ([04](04-RBAC-AND-AUTHORIZATION.md)) |
| Broken object-level authz | No `TASK` scope ⇒ permissions are uniform per project; filters compile with the permission predicate injected, not supplied ([27](27-FILTER-AND-SAVED-VIEW-DSL.md)) |
| Credential theft | Argon2id for **passwords**; tokens and sessions are selector/verifier with a per-row-salted verifier hash and **no server-held key** (ADR-032); opaque revocable sessions, never cached; MFA ([40](40-IDENTITY-AUTH-AND-SESSION.md)) |
| Session hijack | `HttpOnly` `Secure` `SameSite`; rotation on privilege change; CSRF double-submit |
| Account enumeration | Identical responses for login, reset, and invite regardless of account existence |
| SQL injection | All SQL is `sqlx::query!` with bind parameters; filter compiler emits no user-derived strings; property-tested |
| XSS | Markdown sanitized; attachments served from a **separate origin**; strict CSP; no `unsafe-eval` |
| SSRF | Plugin and webhook egress allow-listed; private ranges blocked; redirects not followed off-list |
| Malicious upload | Magic-byte type verification; malware scan before visibility; separate origin ([28](28-ATTACHMENT-PIPELINE.md)) |
| Malicious plugin | Scopes + consent + installer ceiling; out-of-process; timeouts; circuit breakers; quotas ([34](34-PLUGIN-AND-EXTENSION-ARCHITECTURE.md)) |
| DoS | Every input bounded ([21](21-API-LIMITS-AND-QUOTAS.md)); cheap-checks-first ordering; load shedding order defined |
| Insider misuse | Audit including `permission.denied`; admin actions audited; export available |
| Supply chain | `cargo-deny` (gated); lockfiles committed; SBOM and build provenance attested per release; base images pinned **by tag, not digest** — see finding 3 |
| Data loss | PITR backups; verified restore drills each phase; staged deletion with grace periods |

| Outbound mail | **Not yet controlled.** Notification subjects carry task titles ([29](29-NOTIFICATIONS-AND-DELIVERY.md)) and `TF_SMTP_PASS` crosses the same connection; STARTTLS and certificate verification are required by decision but not yet implemented (D-046) |

The threat model is reviewed at each phase gate, not written once.

### Review — Phase 0 gate, 2026-08-08

**Conducted by:** Claude (AI agent), against the implemented tree at commit
`580e8b0`. **This is not a substitute for a human security review or a
penetration test**, and it should be countersigned before the Phase 1 gate. It
is a design-and-documentation review: every row above was checked against what
the repository actually contains, and the checkable claims were executed rather
than read.

Six findings. Three are fixed in the table above; three are tracked.

1. **The credential-theft row was stale by one day.** It read "Argon2id at rest;
   tokens stored hashed". ADR-032 replaced that with selector/verifier and
   confined Argon2id to passwords. **Fixed above.**
2. **The cross-tenant row did not mention its own exception.** ADR-032 accepts a
   `SECURITY DEFINER` function that deliberately bypasses the RLS backstop for
   pre-workspace lookup. A threat model is the first place someone looks for
   "what protects tenant data", so an unlisted hole is the worst place for one.
   **Fixed above**, with the gate condition named.
3. **"Pinned base images" was not true.** `rust:1.96-slim-bookworm` and
   `gcr.io/distroless/cc-debian12:nonroot` are **mutable tags**: the content
   behind them can change without any change here, which is the supply-chain
   substitution this row exists to prevent. Digest pinning is the fix, and the
   cost — a bot or a human must bump digests for security updates — is why it is
   recorded as a decision (**D-048**) rather than changed unilaterally. Row
   amended to say what is actually true today.
4. **Outbound mail was absent from the model entirely.** Notification subjects
   carry task titles and the relay password crosses the same connection. **Row
   added**; controls tracked as D-046.
5. **Verified as claimed, not assumed:** `cargo-deny` runs and passes in CI;
   `Cargo.lock` and `pnpm-lock.yaml` are committed; `release.yml` emits an SBOM
   and attests build provenance; the runtime image is distroless and non-root.
6. **One control became real since the model was written.** "No `TASK` scope ⇒
   permissions are uniform per project" is now structural rather than
   documented: `ScopeType` has no `Task` variant and `casual-task-authz` cannot
   express one (C-003). Most other rows remain **designed, not implemented** —
   XSS, SSRF, malicious upload and malicious plugin all await Phases 1–3, and
   the model should not be read as describing running defences.

## Security baseline

- TLS 1.3; HSTS with preload.
- CSP: `default-src 'self'`, no `unsafe-eval`, no `unsafe-inline` scripts,
  frame-ancestors restricted, plugin iframes on their own origin.
- `X-Content-Type-Options: nosniff`, `Referrer-Policy: strict-origin-when-cross-origin`,
  `Permissions-Policy` minimal.
- Secrets from the environment or a secret manager — never in the database,
  never in the image, never in a log.
- Dependency and container scanning on every build; a critical CVE blocks release.
- Coordinated disclosure policy in `SECURITY.md`; security fixes ship out of band.
- `unsafe_code = "forbid"`; an exception requires an ADR and a focused review.

## Privacy

- **What is collected**: account identity, authorship, and — in audit only — IP
  and user agent.
- **Why IP/UA are retained** (ADR-025): incident investigation is not possible
  without them. This is stated in user-facing documentation, not buried.
- **Retention**: audit 400 days by default, workspace-configurable within a 90-day
  floor; activity for the project lifetime; notifications 90 days; outbox 7 days
  after dispatch.
- **Export**: workspace data and audit export before any retention drop.
- **Deletion**: a deleted user is anonymized in place (ADR-026) — a tombstone
  account, email nulled, PII scrubbed, foreign keys intact. Erasing authored
  history to remove one person would destroy the audit trail for everyone else.
  This position is documented for data-protection review rather than assumed.
- **No third-party analytics in the self-hosted build.** None. A self-hoster's
  data does not leave their infrastructure.

## Testing strategy

| Level | Scope | Where |
| --- | --- | --- |
| Unit | pure logic: resolver, filter compiler, rank algebra, cycle check | every crate |
| Property | additivity, isolation, cursor completeness, compiler/evaluator agreement, no-injection | `-authz`, `-search`, `-task` |
| Integration | real PostgreSQL via testcontainers; transactional atomicity | `-persistence`, `-app` |
| Contract | OpenAPI diff; event schema; plugin contract | CI |
| Performance | `EXPLAIN` no-seq-scan; latency at reference corpus | nightly + PR subset |
| Security | escalation suite, cross-tenant suite, enumeration, fuzzing | CI |
| E2E | core flows + keyboard-only | Playwright |
| Accessibility | axe automated + manual keyboard pass per release | CI + release |

**Golden fixtures** for the permission matrix and event payloads: a change that
shifts a cell must shift it in the fixture, in the same PR, visibly in review.
This is what stops a subtle authorization regression from passing as "tests
updated."

**Property tests over example tests** where an invariant exists. "Adding a grant
never removes a permission" is worth more than fifty hand-written cases, because
it holds for inputs nobody thought of.

## Compatibility contract

| Surface | Guarantee |
| --- | --- |
| REST API | `/v1` stable; additive changes only; breaking ⇒ `/v2` with an overlap window |
| Event payloads | per-type `schema_version`; both delivered during deprecation |
| Plugin contract | semver, independent of app version (ADR-015) |
| Database schema | forward-only; expand→migrate→contract; downgrade-safe within one minor |
| Error codes | append-only, never reused |
| Cursors | opaque; internal shape free to change |
| Saved view / automation JSON | versioned; migrated on read |

**Additive is safe; removal is not.** Clients must tolerate unknown response
fields — stated in the contract and verified by a client-compat test.

## Browser and platform support

Detail in [18](18-SUPPORT-MATRIX.md). Summary: last two major versions of Chrome,
Edge, Firefox, and Safari; no IE; no polyfills for browsers outside the matrix.

## Operational quality

- **Backups**: continuous WAL archiving + daily base backup; PITR.
- **Restore drills each phase**, timed, into a scratch environment. An untested
  backup is a hypothesis.
- **Migrations** are timed in CI against production-shaped data; a migration that
  would lock `task` for more than 1 s fails the build.
- **Rollback**: every release must be rollback-safe for one version, which is
  what expand→migrate→contract buys.
- **Runbooks** for: outbox lag, DLQ growth, plugin circuit storms, search
  projection lag, database failover ([46](46-OBSERVABILITY-AND-OPERATIONS.md)).

## Licensing and clean-room

- **Apache-2.0**, including the patent grant.
- `cargo-deny` permits only Apache-2.0-compatible dependency licenses; a
  copyleft dependency fails the build.
- **Clean-room**: no source, schema, template, asset, or string from OrangeScrum
  or any other tracker enters this repository. Competitive study is of *published
  behaviour and documentation*, recorded with dates
  ([12](12-COMPETITIVE-ANALYSIS.md)). Contributors acknowledge this in
  `CONTRIBUTING.md`.
- Third-party attributions maintained; SBOM published per release.
