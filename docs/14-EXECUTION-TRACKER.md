# 14 — Execution Tracker

Live state of all work. Every non-trivial change gets a row before it is built
([11](11-DESIGN-FIRST-PROCESS.md)), and the row moves as it progresses.

**Last updated: 2026-08-08.**

## Status vocabulary (controlled)

| Status | Meaning |
| --- | --- |
| `Designed` | The design note is written and final. Not built. |
| `Accepted` | Design final **and** its ADRs Accepted. Ready to build. |
| `Building` | In progress. |
| `Built` | Merged, tests passing. |
| `Gated` | Built and its acceptance gates pass in CI. |
| `Blocked` | Waiting on a named dependency. |
| `Deferred` | Deliberately postponed, with a reason. |

`Gated` — not `Built` — is what "done" means. Code that passes its own tests but
has no gate protecting it will regress unnoticed.

## ID scheme

`D-###` design · `F-###` foundation (Phase 0) · `C-###` core (Phase 1) ·
`A-###` admin (Phase 2) · `P-###` platform (Phase 3) · `V-###` advanced (Phase 4).

Stable and never reused.

## Design (D)

The documentation phase. All complete unless noted.

| ID | Item | Doc | Status |
| --- | --- | --- | --- |
| D-001 | Outcome & requirements | [01](01-ORD.md) | Designed |
| D-002 | Target architecture | [02](02-ARCHITECTURE.md) | Designed |
| D-003 | Domain model & invariants | [03](03-DOMAIN-MODEL.md) | Designed |
| D-004 | **RBAC resolution algorithm** | [04](04-RBAC-AND-AUTHORIZATION.md) | Designed |
| D-005 | API & SSE contract | [05](05-API-SPEC.md) | Designed |
| D-006 | Roadmap & exit gates | [06](06-ROADMAP-AND-DELIVERY.md) | Designed |
| D-007 | Quality, security, threat model | [07](07-QUALITY-SECURITY-AND-COMPATIBILITY.md) | Designed |
| D-008 | ADR register | [08](08-ADR-REGISTER.md) | Designed |
| D-009 | Crate layer division | [19](19-WORKSPACE-SCAFFOLD-DESIGN.md) | Designed |
| D-010 | Error-code registry | [20](20-ERROR-CODE-REGISTRY.md) | Designed |
| D-011 | Limits & quotas | [21](21-API-LIMITS-AND-QUOTAS.md) | Designed |
| D-012 | Database schema | [22](22-DATABASE-SCHEMA.md) | Designed |
| D-013 | Workflow & state machine | [23](23-WORKFLOW-AND-STATE-MACHINE.md) | Designed |
| D-014 | Concurrency & idempotency | [24](24-CONCURRENCY-AND-IDEMPOTENCY.md) | Designed |
| D-015 | Events, outbox, audit | [25](25-EVENTS-OUTBOX-AND-AUDIT.md) | Designed |
| D-016 | **Search & index contract** | [26](26-SEARCH-INDEXING-AND-QUERY.md) | Designed |
| D-017 | Filter grammar & saved views | [27](27-FILTER-AND-SAVED-VIEW-DSL.md) | Designed |
| D-018 | Attachment pipeline | [28](28-ATTACHMENT-PIPELINE.md) | Designed |
| D-019 | Notifications | [29](29-NOTIFICATIONS-AND-DELIVERY.md) | Designed |
| D-020 | Performance & capacity | [30](30-PERFORMANCE-AND-CAPACITY-TARGETS.md) | Designed |
| D-021 | Tenancy & isolation | [32](32-TENANCY-AND-ISOLATION.md) | Designed |
| D-022 | **Plugin & extension architecture** | [34](34-PLUGIN-AND-EXTENSION-ARCHITECTURE.md) | Designed |
| D-023 | Automation rules | [36](36-AUTOMATION-RULES-DESIGN.md) | Designed |
| D-024 | Identity, auth, session | [40](40-IDENTITY-AUTH-AND-SESSION.md) | Designed |
| D-025 | Frontend architecture | [42](42-FRONTEND-ARCHITECTURE.md) | Designed |
| D-026 | Observability & operations | [46](46-OBSERVABILITY-AND-OPERATIONS.md) | Designed |
| D-027 | Deployment profiles | [48](48-DEPLOYMENT-PROFILES.md) | Designed |
| D-028 | Competitive analysis | [12](12-COMPETITIVE-ANALYSIS.md) | Designed |
| D-029 | Process, glossary, gates, maintenance | [11](11-DESIGN-FIRST-PROCESS.md), [15](15-CI-AND-RELEASE-GATES.md), [16](16-DOCUMENTATION-MAINTENANCE.md), [17](17-GLOSSARY.md) | Designed |
| D-030 | Support matrix | [18](18-SUPPORT-MATRIX.md) | Designed |
| D-031 | Repository & contribution | [09](09-REPOSITORY-AND-CONTRIBUTION.md), [10](10-PROJECT-GOAL-AND-STANDARDS.md) | Designed |
| D-035 | **Reporting, export & dashboards** | [38](38-REPORTING-EXPORT-AND-DASHBOARDS.md) | Designed |
| D-036 | Runbooks | [50](50-RUNBOOKS.md) | Designed |
| D-037 | Deployment guide | [52](52-DEPLOYMENT-GUIDE.md) | Designed |
| D-032 | **Auth mechanism: credential lookup, session storage, plugin principal** | [40](40-IDENTITY-AUTH-AND-SESSION.md) | **Accepted** — ADR-032 |
| D-033 | Custom-field value storage | — | **Deferred** — Accept before Phase 3 |
| D-034 | Multi-region / data residency | — | **Deferred** — no commitment until designed |
| D-038 | **Outbox dispatch: claim protocol, per-consumer state, ordering** | [25](25-EVENTS-OUTBOX-AND-AUDIT.md) | **Accepted** — claim → commit → HTTP → record |
| D-039 | Connection pool sizing, acquisition timeout, exhaustion behaviour | [30](30-PERFORMANCE-AND-CAPACITY-TARGETS.md) | **Accepted** — bounded pool, short acquire timeout, 503 |
| D-040 | Queue bounds and full-queue policy | [24](24-CONCURRENCY-AND-IDEMPOTENCY.md) | **Consumed** — written into docs/24 §Every bound names its overflow policy, C-011 |
| D-041 | Cancellation and graceful shutdown | [24](24-CONCURRENCY-AND-IDEMPOTENCY.md) | **Consumed** — written into docs/24 §Cancellation and graceful shutdown, C-011 |
| D-042 | Rate-limit attribution, and expiry for investigation admissions | [46](46-OBSERVABILITY-AND-OPERATIONS.md) | **Accepted** — no workspace ids in labels; admissions expire |
| D-043 | **Full-text search under RLS sequentially scans at reference scale** | [26](26-SEARCH-INDEXING-AND-QUERY.md) | **Accepted** — keep RLS; try a tenant-filtered projection first |
| D-044 | MSRV and toolchain-pin ADR | [08](08-ADR-REGISTER.md) | **Accepted** — ADR-031 |
| D-045 | SSE vs WebSocket for bidirectional features | [05](05-API-SPEC.md) | **Deferred** — only if a feature needs client→server streaming |
| D-046 | **Outbound mail security: STARTTLS requirement and certificate verification** | [29](29-NOTIFICATIONS-AND-DELIVERY.md) | **Accepted** — STARTTLS + certificate/hostname verification |
| D-047 | **What `outbox_lag_seconds` measures, and how a cache-hit ratio is exported** | [46](46-OBSERVABILITY-AND-OPERATIONS.md) | **Consumed** (lag half) — gauge, age of oldest actionable pending delivery, C-011. The cache-hit ratio half stays open until C-003's cache exists. |
| D-048 | Pin base images by digest rather than tag | [07](07-QUALITY-SECURITY-AND-COMPATIBILITY.md) | **Blocked** — Accept before the first release |
| D-049 | Is assigning a role distinct from authoring one at workspace scope? | [04](04-RBAC-AND-AUTHORIZATION.md) | **Consumed** — yes, distinct: `role.assign` vs `role.manage`, migration 0015 |
| D-052 | Whether a shared test-support crate should exist | [19](19-WORKSPACE-SCAFFOLD-DESIGN.md) | **Open** — surfaced by C-011 |
| D-050 | Database TLS, and the `CDLA-Permissive-2.0` licence it requires | [52](52-DEPLOYMENT-GUIDE.md) | **Consumed** — no database TLS; trusted network required, and the licence gate is what holds it |
| D-051 | How `key` (`WR-125`) is filtered, given it spans two tables | [27](27-FILTER-AND-SAVED-VIEW-DSL.md) | **Blocked** — Accept before C-013 |

Eight of those are new. **D-038** to **D-043** were opened by Phase 0 audits of
the concurrency, async, and observability design; **D-044** and **D-045** were
already listed as pending ADRs in [08](08-ADR-REGISTER.md) §Pending and had no
tracker row at all, which AGENTS.md forbids — an untracked decision is one
nobody is accountable for. They are recorded rather than resolved because each
is a decision, and AGENTS.md forbids settling one silently in an
implementation:

- **D-038.** [25](25-EVENTS-OUTBOX-AND-AUDIT.md) describes dispatch as holding a
  transaction open across consumer I/O, which pins a connection for the duration
  of an HTTP call to a webhook endpoint. It specifies six consumers that are
  "independently retried", but `outbox_event` carries one row-level state for all
  of them, so one consumer's failure and another's success have nowhere to be
  recorded separately. It also claims per-aggregate ordering with no mechanism
  that provides it. Three related holes, one decision.
- **D-039.** The exponential-backoff ladder in [25](25-EVENTS-OUTBOX-AND-AUDIT.md)
  is unimplementable against the committed schema: `outbox_event` (migration
  0007) has no next-attempt column, so "retry in 4 minutes" cannot be expressed
  and the dispatch poll cannot exclude a row that is waiting. This one is
  schema-visible and is cheapest to settle before the table has data in it.
- **D-040.** [24](24-CONCURRENCY-AND-IDEMPOTENCY.md) says "all queues are
  bounded" without naming a bound or what happens when one is full. The bound is
  now enforced mechanically — `clippy.toml` rejects every unbounded-channel
  constructor by resolved path — but "bounded" without a policy for the full case
  just moves the failure from an out-of-memory crash to an unspecified one.
- **D-042.** [46](46-OBSERVABILITY-AND-OPERATIONS.md) contradicts itself: §Domain
  metrics writes `rate_limit_hits_total` "by workspace", and §Cardinality
  discipline forbids a raw `workspace_id` label on any metric. The registry
  resolved it in favour of the discipline — a hashed bucket, which answers
  "is throttling concentrated?" and not "which tenant?" — and the code claimed
  that resolution was "recorded as an open question", which it was not. It is
  now. Second half of the same row: §Cardinality discipline permits per-tenant
  labels "enabled temporarily", and the allow-list enforces *small* but not
  *temporarily*. There is no clock, so an admission lasts forever. Deciding the
  expiry is a design choice, not an implementation detail.
- **D-043.** Measured, not predicted. `tests/explain/queries/11` already
  documented that `tsvector @@ tsquery` resolves to `ts_match_vq`, which is not
  `LEAKPROOF`, so PostgreSQL will not evaluate it before the row-security qual
  and therefore cannot use `task_search_gin` as an index qual under RLS. It
  concluded "it is not a Seq Scan, so this gate passes it." That conclusion is
  corpus-size-dependent, and it is false at the corpus the product targets. On a
  loaded 2,000,000-task corpus, same query, same 6%-selective term, same
  instance:

  | connected as | RLS | plan |
  | --- | --- | --- |
  | `taskforge_app` | applied | **`Parallel Seq Scan on task_search`** |
  | owner | not applied | `Bitmap Index Scan on task_search_gin` |

  RLS is the only difference. So the product's own search path performs the
  sequential scan on a tenant-scale table that [26](26-SEARCH-INDEXING-AND-QUERY.md)
  NFR-5 and ADR-011 forbid, and the `explain-no-seq-scan` gate cannot see it,
  because it runs at ~109k rows where the same query still resolves to an index
  scan. **A green gate does not mean the rule holds at reference scale.**

  This touches ADR-011, ADR-014, and ADR-020 and is a decision, not a fix:
  the options (a `LEAKPROOF` wrapper, a security-definer function, dropping RLS
  on the projection table in favour of an explicit predicate, or the dedicated
  search engine ADR-014 already names as its tripwire) trade tenant-isolation
  guarantees against query plans, and that trade is exactly what an ADR is for.
  Separately: raising the gate's corpus, or running it at reference scale
  nightly, is what would have caught this.
- **D-032** is narrower than its old title suggested and sharper than the ADR
  register described. [40](40-IDENTITY-AUTH-AND-SESSION.md) is a finished
  document: cookie-vs-bearer, the deliberate absence of a refresh strategy, OIDC
  claim mapping, lifetimes, rotation, CSRF, Argon2id parameters, the token
  formats, and eight acceptance gates are all decided. What is open is the
  mechanism beneath it, where that prose meets the **already-`Gated`** schema.
  Four contradictions, each verified against `migrations/`:

  1. **A salted hash cannot be looked up.** `api_token.token_hash text NOT NULL
     UNIQUE` (migration 0008) and [21](21-API-LIMITS-AND-QUOTAS.md):134
     ("authentication — cheap: one indexed read") both require a *deterministic*
     digest. [40](40-IDENTITY-AUTH-AND-SESSION.md) says tokens are "hashed at
     rest" without naming an algorithm, and specifies Argon2id two lines above
     for *passwords*. Argon2id is salted, so an implementer who reaches for the
     nearest password hasher gets a token nobody can find. Which digest, and
     keyed with what, is the decision.
  2. **`TF_SECRET_KEY` has no stated job.** [48](48-DEPLOYMENT-PROFILES.md):101
     calls it "session/cookie signing", and
     [40](40-IDENTITY-AUTH-AND-SESSION.md):26 specifies a plain opaque 256-bit
     cookie, which has nothing to sign. Either the cookie is signed and docs/40
     is incomplete, or the key is for something else (token keying, CSRF) and
     docs/48 is wrong.
  3. **The Redis cache can outlive a revocation.** "Revocation is immediate:
     delete the row" (:44) is the *entire* stated reason for rejecting JWTs
     (:31-36). A read-through cache (:38) reintroduces exactly the staleness
     window that argument rejects, unless invalidation is specified.
  4. **A plugin token has no principal.** `principal_type` is
     `ENUM ('USER','TEAM','SERVICE_ACCOUNT')` (migration 0001) and
     [40](40-IDENTITY-AUTH-AND-SESSION.md):132 specifies a per-installation
     plugin token. An enum change to a Gated schema is cheapest now.

  Beyond the contradictions: **no auth state has a table at all.** Migrations
  0001–0012 are `Gated` and define no session, credential, MFA-factor,
  recovery-code, reset-token, invitation, or SSO-connection table, and
  [05](05-API-SPEC.md) lists no auth endpoint. Separately, `user_account` is the
  only table without `workspace_id` while `enforce_sso`, MFA enforcement and
  `allowed_domains` are per-workspace — so which workspace's policy governs a
  login, before any workspace is known, is undecided.

  **Accepted as ADR-032, with amendments** — the resolutions are in
  [40](40-IDENTITY-AUTH-AND-SESSION.md) §Mechanism. Two proposals were rejected
  in favour of better ones, and both rejections removed a cost:

  - **Selector/verifier, not a keyed HMAC.** The pepper would have made a secret
    outside the database load-bearing for every authentication — lose it and
    every session dies, rotate it and they die without a versioning window —
    which forced `hash_key_id` onto two tables and key custody into the
    runbooks. Selector/verifier gets the same dump-resistance for a longer
    token and no key at all.
  - **`principal_type` is not extended.** A plugin installation authenticates
    but is not something a role is assigned to. Making it a principal would have
    put it in the resolver's principal set and invited grants assigned directly
    to installations — a second authority model reaching the same resources.
    [04](04-RBAC-AND-AUTHORIZATION.md) stays the only answer to "who may do
    what", and the enum needs no migration.

  Carried into C-001: `api_token.token_hash` becomes `token_selector` +
  `verifier_hash`, and the auth-storage tables land with a written exemption in
  migration 0010's block. The `SECURITY DEFINER` seam is accepted **on the
  condition** that the F-015 gate is extended to assert the function's
  definition, not only its tables.
- **D-044.** [08](08-ADR-REGISTER.md) §Pending lists "MSRV and toolchain pin" —
  once the workspace is scaffolded". The workspace *is* scaffolded, so the ADR
  is due. Until it exists, `rust-version = "1.90.0"` in `Cargo.toml` and the
  `platform` job's MSRV matrix entry are a number nobody agreed to; both used to
  cite **D-032**, which is the auth item, so the pointer was wrong as well as
  dangling. F-002 cannot be `Gated` until this lands.
- **D-046.** [07](07-QUALITY-SECURITY-AND-COMPATIBILITY.md) specifies TLS 1.3
  for traffic *into* the system. Nothing specifies anything about traffic *out*
  of it to an SMTP relay: not whether STARTTLS is required, not whether the
  relay's certificate is verified, not what happens when a relay offers no TLS
  at all. Email does not appear in the threat model. This is not academic —
  [29](29-NOTIFICATIONS-AND-DELIVERY.md) §Email content puts the task title in
  the subject line (`[WR-125] Task title`), so every notification carries tenant
  content, and `TF_SMTP_PASS` crosses the same connection. Silently falling back
  to cleartext is the failure mode to design out, and which way to fail is a
  security decision rather than a client-library default to inherit.
- **D-047.** Two instrument kinds do not match what
  [46](46-OBSERVABILITY-AND-OPERATIONS.md) asks the metric to answer, and one of
  them is the **primary health signal**, with a paging alert and an SLO resting
  on it.

  `outbox_lag_seconds` is registered as a Histogram and its help text reads
  "age of the oldest undispatched outbox event". Those are different
  quantities. Sampling a point-in-time oldest-age into a histogram yields a
  distribution *of scrapes*, not of events — so §Alerts' "p95 > 30 s for 5 min"
  and §SLOs' "outbox lag < 5 s, 99%" would both be computed over the wrong
  population. Either it is per-event dispatch lag observed at dispatch
  (histogram, and the help text is wrong), or it is the oldest pending age
  (gauge, and a companion series is needed for the percentiles). Which one is a
  decision about what the page should fire on.

  Second, `authz_cache_hit_ratio` is a Gauge. A pre-computed ratio cannot be
  averaged across replicas or re-windowed in a query; the aggregatable shape is
  two counters with the ratio computed at query time. That changes a metric
  docs/46 names, so it is not a silent refactor.
- **D-044 is Accepted as ADR-031**, and the number moved. The declared
  `rust-version = "1.90.0"` was justified by nothing; the floor was measured by
  lowering the declaration and building against installed toolchains:

  | toolchain | result |
  | --- | --- |
  | 1.85.0 | fails — `time` 0.3.55, `time-core` 0.1.9 and `time-macros` 0.2.32 each require 1.88.0 |
  | **1.88.0** | builds, and all 154 tests pass — **now the declared MSRV** |
  | 1.90.0 | the old declaration, two releases above anything requiring it |

  The rule ADR-031 sets is that the MSRV *is* the measured floor, so raising it
  requires a PR to name the crate and version responsible. The `platform`
  matrix now tests 1.88.0 — the MSRV itself rather than a number near it — and
  `rust-toolchain.toml` still pins current stable, which is a deliberately
  different number.
- **D-045.** [08](08-ADR-REGISTER.md) §Pending also lists SSE vs WebSocket.
  `Deferred` rather than `Blocked`: SSE is the Phase 1 decision and no feature
  yet needs client→server streaming, so this only becomes live if one does.
- **D-041.** No document describes cancellation or graceful shutdown: what
  happens to an in-flight request, a half-dispatched outbox row, or a held
  advisory lock when a pod is terminated. Retrofitting cancellation through a
  codebase that never considered it is materially harder than designing it in.

- **D-052.** C-011's acceptance gate lives in `casual-task-worker` and needs the
  same PostgreSQL-container harness `casual-task-persistence` already has.
  Integration tests are per-crate binaries, so it was **copied**. Two copies
  drift, and the drift would be silent — a worker test passing against a schema
  the persistence tests no longer describe. Consolidating it into a shared dev
  crate changes the workspace dependency DAG that
  [19](19-WORKSPACE-SCAFFOLD-DESIGN.md) fixes and `casual-task-lint` enforces,
  which is a design decision rather than a refactor. The duplication is marked
  in both files rather than left to be discovered.

**Eight decisions were Accepted on 2026-08-08.** Their design notes are rewritten
as each is consumed, so the change lands with the code that proves it.

**D-038 is now written into [25](25-EVENTS-OUTBOX-AND-AUDIT.md) §Dispatch**, which
until now still described holding a database transaction open across consumer
HTTP I/O — the shape D-038 rejected. Anyone implementing C-011 from that section
would have built it. It now specifies claim → commit → HTTP → record, the claim
expiry that makes a crashed worker recoverable, per-consumer delivery state, and
the `next_attempt_at` the backoff ladder needs. The two schema changes it named
as missing are now made — migration
[0013](../migrations/0013_outbox_delivery.sql), landed with C-011 below.

The remaining seven are additive rather than contradictory: pool bounds and 503
on exhaustion (D-039, [30](30-PERFORMANCE-AND-CAPACITY-TARGETS.md)), queue
bounds with an explicit overflow policy (D-040,
[24](24-CONCURRENCY-AND-IDEMPOTENCY.md)), bounded drain on shutdown (D-041,
[48](48-DEPLOYMENT-PROFILES.md)), no workspace ids in metric labels and expiring
investigation admissions (D-042), a tenant-filtered search projection tried
before weakening RLS (D-043), STARTTLS with certificate and hostname
verification (D-046, [29](29-NOTIFICATIONS-AND-DELIVERY.md)), and
`outbox_lag_seconds` as a gauge over the oldest *actionable* pending event
(D-047, [46](46-OBSERVABILITY-AND-OPERATIONS.md)).

> The README's per-phase progress table is **generated from the tables below**
> by `scripts/phase-progress.py` and checked in CI. Changing a status here moves
> the README; a README edited by hand fails the build. That is deliberate — the
> status badge said "Phase 0" for a week after Phase 0 closed, because a number
> written in a second place has no reason to change when the first place does.

## Phase 0 — Foundation (F)

| ID | Item | Status | Blocked by |
| --- | --- | --- | --- |
| F-001 | Cargo workspace + all crates with declared edges | **Gated** | — |
| F-002 | Toolchain pin, MSRV ADR, `deny.toml` | **Gated** | — |
| F-003 | CI: fmt, clippy, deny, nextest | **Gated** | F-001 |
| F-004 | Custom architecture lints ([15](15-CI-AND-RELEASE-GATES.md)) | **Gated** | F-001 |
| F-005 | PostgreSQL testcontainers harness + migration runner | **Gated** | — |
| F-006 | `tools/casual-task-seed` reference corpus | **Gated** | F-005 |
| F-007 | `tools/casual-task-loadtest` + baselines | `Built` | F-006 |
| F-008 | `EXPLAIN` no-seq-scan harness | **Gated** | F-006 |
| F-009 | Observability skeleton | `Built` | F-001 |
| F-010 | Docker Compose dev profile | **Gated** | — |
| F-011 | Governance files, Apache-2.0, AGENTS.md | `Built` | — |
| F-012 | **Bundle floor measurement** (ADR-024) | **Gated** | — |
| F-013 | Threat model review | `Built` | — |
| F-014 | Runbooks (initial set) | `Built` | F-009 |
| F-015 | Migrations + application role + schema verification gate | **Gated** | F-005 |
| F-016 | Container image, deployment compose, deployment guide | **Gated** | F-015 |

## Phase 1 — Core (C)

| ID | Item | Status |
| --- | --- | --- |
| C-001 | Identity, sessions, MFA, invitations | Accepted |
| C-002 | Workspace, membership, teams | Accepted |
| C-003 | **Permission resolver + `/explain`** | `Building` |
| C-004 | Permission matrix + escalation suites | `Building` |
| C-005 | Cross-tenant property suite | `Building` |
| C-006 | Projects, membership, visibility | Accepted |
| C-007 | Default workflow + transitions | `Building` |
| C-008 | Task CRUD, assignees, tags | Accepted |
| C-009 | Comments | Accepted |
| C-010 | Attachment pipeline | Accepted |
| C-011 | Activity + audit + **outbox** | `Building` |
| C-012 | Filter grammar + compiler | `Building` |
| C-013 | Search projection + full-text | Accepted |
| C-014 | Cursor pagination | `Building` |
| C-015 | SSE + fan-out | Accepted |
| C-016 | Notifications (in-app + email) | Accepted |
| C-017 | Extension point registry (core panels only) | Accepted |
| C-018 | Web shell, board, list, My Work, drawer, palette | Accepted |
| C-019 | Bundle + a11y gates wired | Accepted |

- **D-050** was opened by a failing gate rather than by reading anything.
  Adding `sqlx` with `tls-rustls` pulled in `webpki-roots` — Mozilla's CA bundle,
  licensed **CDLA-Permissive-2.0**, which `deny.toml` does not allow — and
  `cargo deny check licenses` went red. Widening the allow-list is a licensing
  policy decision, not a build fix, so the feature is off for now: nothing
  connects to a remote database yet, and the tests talk to a local container.
  It goes back on when the API needs it, with the licence decided deliberately
  at that point rather than smuggled in beside a test harness.

**The read path executes.** Filter → validate → resolve → compile → run, against
a real PostgreSQL 16 as `taskforge_app` with RLS applied. Three tests assert the
rows that come back, not the SQL that goes in.

That is the difference that mattered: every earlier test asserted the compiler's
*output*, which proves shape and nothing about whether PostgreSQL accepts it.
Running it immediately produced `operator does not exist: task_state = text` —
a bound text parameter against an enum column. The fix is to cast the
**parameter**, never the column: `t.state = $3::task_state` uses the index,
`t.state::text = $3` does not and would turn every filtered list into the
sequential scan NFR-5 forbids. Verified by removing the casts and watching the
suite fail with that error.

**D-051 was opened by the same exercise.** `key` is `WR-125` — `project.key` and
`task.number` concatenated, in two tables — and docs/27 lists it as filterable
while the schema has no column to compare against. It now compiles to a
predicate matching **nothing**, deliberately: a `key` filter returning no rows is
visibly wrong to whoever ran it, where comparing against `t.number` alone would
match `WR-125` and `OPS-125` identically and look right.

**Cursor pagination is in** (C-014). The sortable set is closed and **smaller
than the filterable one** — a field can be filterable without being sortable,
and conflating them is the mistake worth guarding: `title` filters through a
trigram index, but *ordering* by it would sort the whole result set with nothing
behind it. A test asserts six filterable fields are refused as sorts with
`TF-QRY-0002`.

Pages are keysets, never `OFFSET`. The resume predicate is a **row-value**
comparison — `(key, id) < ($k, $id)` — rather than the expanded
`key < $k OR (key = $k AND id < $id)`, because PostgreSQL drives a composite
index directly from the row-value form and often cannot from the expanded one.
That is the difference between a keyset page and a scan.

Three things a test pins because each is silently wrong rather than loudly
broken: the id tiebreaker is always present (without it, ties in `updated_at` —
constant on bulk operations — make a page repeat or skip rows); the comparison
follows the sort direction, and inverting it serves the same page forever; and
`LIMIT` is n + 1, which answers "is there a next page" without a `COUNT` over
the result set.

**Symbolic resolution is in** (C-012). `@me`, `@my_teams`, `@unassigned`,
`@today`, `@tomorrow`, `@start_of_week` and the `+7d` / `-3mo` forms resolve at
evaluation, which is what keeps a stored `@me` correct for whoever opens the
view — docs/27: "A view that hardcoded a user id would be shareable but wrong."

The timezone bug docs/27 names is closed by the type. `Context` requires a
`UtcOffset` and has **no default**, so a caller cannot accidentally resolve
`@today` against the server's midnight. A test resolves the same instant for an
actor at UTC-7 and one at UTC+12 and asserts they get **different** days —
verified by reverting to server-local and watching it fail.

Two smaller decisions are stated rather than hidden: `@start_of_week` is Monday
per ISO 8601, which is a product choice and not a computation; and `-3mo` is
ninety days, because calendar months make the same filter land on a different
day depending on which months it crosses. An unknown symbol is **refused**, not
passed through — a typo'd `@tomorow` reaching the database as a literal would
compare a timestamp against a string and fail somewhere far worse.

**The filter compiler is in** (C-012, second half). A validated AST compiles to
parameterized SQL in `casual-task-persistence`, which is where docs/19 requires
all SQL to live — so the AST crate still cannot emit a fragment, and the split
is what makes docs/27's "no path from user input to SQL" structural.

Two of docs/27's requirements are enforced by the signature rather than by
review. The permission filter is **injected**, not supplied: `compile` takes an
`AuthorizedProjectSet` and there is no overload without one, so a query missing
its tenant predicate does not compile. And many-to-many fields (`assignee`,
`tag`) emit `EXISTS` rather than `JOIN` — a join makes a task with two matching
tags appear twice, forcing `DISTINCT`, which breaks keyset pagination because
the cursor's `(updated_at, id)` stops being a total order.

The injection gate docs/15 §Security names is in, and it is stated as the
property that actually holds. "The SQL contains no user text" is wrong in both
directions — `$1` and `t.workspace_id` are hostile-looking inputs the SQL
legitimately contains, and a leak could still take a form a substring search
misses. The assertion is instead that compiling the same filter with **any**
value produces byte-identical SQL. Verified by interpolating a value instead of
binding it and watching the test name the string that leaked.

**C-005 is `Building`, and the persistence seam is in.**
`casual-task-persistence::Scoped` is the only door to tenant data: it applies a
`WorkspaceScope` to a transaction as the GUC migration 0010's policy reads, and
a repository cannot hold one without that having happened. The setting is
transaction-local — a session-level one would outlive the request and the next
pooled checkout would inherit another tenant's scope, which is a leak that only
appears under load.

Three tests assert it against a real PostgreSQL 16 as the **non-superuser**
role, because RLS is inert for a superuser and the same assertions run as the
owner would prove nothing: a scoped transaction sees exactly its own tenant, an
unscoped one sees **nothing rather than everything**, and the scope does not
survive a commit on a single-connection pool.

A fourth runs without Docker and is the one most likely to earn its keep: it
asserts that migration 0010 builds its policy with the exact setting name the
code sets. Drift there is silent — every scoped read returns zero rows and
nothing errors. Writing it loosely would have missed the case it exists for, so
it matches the doubled-quoted form rather than a substring; verified by
truncating the constant to `taskforge.workspace` and watching it fail.

**C-012 is `Building`.** The AST and the closed field set are implemented in
`casual-task-search`: two node kinds and no more, nineteen fields as enum
variants, per-field operator tables, value-shape checking, and the docs/21
bounds (32 clauses, depth 4) reported with their already-registered `TF-QRY-*`
codes.

ADR-011 is enforced by the type rather than by a check — a field the design
record has not listed, indexed, `EXPLAIN`-asserted and given a UI control has no
variant, so it cannot be named. The operator tables are per-field rather than
per-type on purpose: `reporter` takes no `is_empty` (a task always has one),
`tag` takes no `eq` (it is a set), `key` takes `starts_with` while `title` takes
`contains`. Deriving them from the type would have quietly widened the surface,
and a test asserts each of those asymmetries.

**The SQL compiler is deliberately not here.** docs/19 puts all SQL in
`casual-task-persistence` and the architecture lint fails the build otherwise —
which is what makes docs/27's "no path from user input to a SQL fragment"
structural: this crate cannot emit one. Compilation, the URL and JSON surfaces,
and symbolic-value resolution (`@me`, `+7d`, and the actor-timezone question
that comes with `@today`) are the remaining work.

**C-007 is `Building`.** The state machine is implemented in
`casual-task-workflow`: statuses permanently mapped to the five states,
transitions including the `from = NULL` wildcard that expresses "cancel from
anywhere" without a row per status, and steps 4–7 of docs/23's fixed validation
order — edge exists, transition permission, required fields, blocking
dependencies — returning the **first** failure, because "the error a user sees
is the most actionable one".

Two invariants are structural rather than checked. A `Status` cannot be built
without a state, so "status is yours; state is ours" cannot be violated by
construction; and a validated transition carries the destination status **and**
its state together, which is the in-memory form of docs/23's guarantee that
`state` is written in the same statement as `status_id`. Construction is
fallible for the same reason: a workflow with no initial status, or two, is a
shape the schema's partial unique index would refuse, so the type refuses it
too.

Steps 1–3 and 8 are deliberately absent — reading a task, resolving the actor's
permissions, and plugin hooks belong to persistence, `casual-task-authz` and
Phase 3. The caller passes their results in, which is what lets the whole state
machine be tested with no database and no runtime.

Still missing before `Gated`: status editing and the status-migration path
(docs/23 §Editing a workflow), and the transition command itself, which needs
the command layer.

**C-004 is `Building`.** Five of the seven escalation controls are implemented
in `casual-task-authz` as `may_assign` and `plugin_ceiling`, each with a test
that *attempts* the exploit rather than asserting about it. The other two are
not, and the module says so rather than skipping them quietly: last-owner
protection is a database constraint checked inside the transaction (docs/04
control 4 says "not just in application code", so a check here would be
advisory and would race), and auditing every grant needs C-011.

Both property tests docs/04 §Acceptance gates names are in: additivity — "the
invariant the whole model rests on" — and cross-workspace isolation. Both are
seeded so a failure names its reproducing case, and both were verified to fail:
injecting a most-specific-wins rule between grants makes additivity red at a
named seed. A third test asserts the generator produces both allows and denies,
because a property test over a generator that never allows anything is
vacuously true.

Still missing before `Gated`: the golden matrix over every permission × role ×
scope, and the no-N+1 and 404-not-403 gates, which need a query layer and
endpoints respectively.

**D-049 and D-050 are settled, and C-001 and C-002 are unblocked.**

**D-049 — assigning is not authoring.** The closed permission set had
`project.role.assign` for assigning inside a project and only `role.manage`
above it, which is also the *authoring* permission. A workspace-level assigner
therefore had to hold the right to author roles — and could mint a role
carrying more than they held, then grant it to themselves. Control 1 forbade the
direct version, so the hole was narrow; it sat exactly where the most privileged
actors are. Migration [0015](../migrations/0015_role_assign_permission.sql) adds
`role.assign`, and `assign_permission_for` uses it above project scope.

Two tests carry it, and both were verified to **fail without the change** rather
than assumed to: an actor holding only `role.assign` can now assign an ordinary
role at workspace scope (previously impossible), and still cannot hand out
`role.manage` (previously the same permission).

**D-050 — no database TLS; the database must be on a trusted network.** Enabling
it in `sqlx` pulls `webpki-roots` (`CDLA-Permissive-2.0`), which `deny.toml`
rejects. The choice was between adding a licence obligation for a capability
nothing currently uses and documenting a constraint every current deployment
already satisfies. The cost is real and now written where an operator will meet
it ([52](52-DEPLOYMENT-GUIDE.md)): **a managed PostgreSQL across a public
network is not a supported deployment today.**

What holds it is not a note. Turning TLS on fails `cargo deny check licenses`
with a named licence, so the decision is revisited by someone reading the
section rather than smuggled in beside an unrelated change. And
`verify-deployment.sh` now asserts the database publishes **no host port** —
`expose`, never `ports` — because a compose file quietly gaining one is the
single change that turns a documented constraint into an unencrypted database
on the internet. The assertion was checked against a deliberately broken compose
file before being trusted.

**C-011's runtime half is in.** The dispatch loop, the dispatcher role, the
retention sweep, and the acceptance gate `docs/25` names.

The loop is claim → commit → deliver → record, arranged so the shape is visible:
`claim_batch` commits before returning and nothing borrowed from its transaction
escapes, so a caller *cannot* hold one across delivery.

**`Dispatcher` is a capability type, and it verifies itself.** The dispatcher
polls every workspace, so it needs to bypass the policy on `outbox_delivery`.
Built on `Scoped` it would have seen nothing — and seen it without erroring,
reporting healthy while delivering silence. `Dispatcher::assume` asks the
database whether the connected role can actually bypass RLS and refuses if not,
naming the role. Wiring the wrong role now fails at startup instead of
succeeding into that silence.

Migration [0014](../migrations/0014_dispatcher_role.sql) creates
`taskforge_dispatcher`: `BYPASSRLS`, `NOSUPERUSER`, and granted on the two
outbox tables and nothing else. `BYPASSRLS` is database-wide, so the grants are
what bound it — bypassing a policy on a table you cannot select from grants
nothing. `verify-deployment.sh` asserts all three directions: it cannot read
`task`, it can read `outbox_delivery`, and it cannot INSERT an `outbox_event`
(which would be manufacturing an event that never happened).

Two things caught while building it:

- The first version of the loop awaited `deliver()` **before** spawning, which
  serialised every delivery behind the previous one and made the semaphore
  decorative — concurrency of one, bounded by a permit nobody contended for.
- Migration 0014 used a bare `CREATE ROLE`, which passes every test that starts
  from an empty database and fails a real deployment on first `up`, because
  `deploy/` creates the role in the entrypoint before migrations run. The
  deployment gate caught it; the schema gate could not have.

D-040 and D-041 are **consumed**, written into
[24](24-CONCURRENCY-AND-IDEMPOTENCY.md) with the bounds this loop actually
enforces rather than as intentions. D-047's lag half is consumed too:
`outbox_lag_seconds` is now a Gauge in the registry and in
[46](46-OBSERVABILITY-AND-OPERATIONS.md), which had flagged the contradiction
itself.

CI now runs `cargo test --workspace -- --ignored` rather than naming one crate.
The step's own comment argued for "every ignored test in the crate, not one
named file"; the same argument applies a level up, and the C-011 gate lives in a
different crate.

Still missing before `Gated`: the six consumers themselves (C-013, C-015,
C-016), the sweep's scheduling under a leader lease, and metric *emission* —
the registry declares the series and `dispatch` computes the readings, but
nothing exports them yet (F-009 is `Built`, not `Gated`).

**C-011 is `Building`.** The transactional write path and the dispatch loop are
implemented, with 11 integration tests against a real PostgreSQL 16 — the
`#[ignore]`d suite CI runs in its `schema` job.

`UnitOfWork::record` writes the activity record, the audit record, the outbox
event, and one delivery row per consumer in the **caller's** transaction. It does
not commit: a unit of work frequently spans more than one aggregate, and a type
that committed on its own could not express that. Two tests carry the ADR-006
guarantee — one that a commit leaves all four, one that a rollback leaves none.
The second is the one that matters; a change whose history has a hole in it is
the failure the outbox exists to prevent.

`dispatch` is deliberately three functions and not one. `claim`, `succeeded` and
`failed` cannot be composed into a call that holds a transaction across consumer
HTTP, which is the shape D-038 rejected. A test proves it rather than the
docstring asserting it: after the claim commits, a second connection takes
`FOR UPDATE` on the claimed row and must not block.

Migration [0013](../migrations/0013_outbox_delivery.sql) adds `outbox_delivery`
— one row per `(event, consumer)` — and **drops** `dispatched_at`, `attempts`
and `last_error` from `outbox_event`. Dropping them was the point: left in
place, a dispatcher updating `outbox_event.dispatched_at` would run without
error, report success, and deliver to none of the six consumers.

Three things the gates caught that review would not have:

- The lag gauge decoded `min(...)` as a plain `f64`. An aggregate over zero rows
  returns one row containing NULL, so it worked with a backlog and failed with
  none — the state a healthy system is in almost all of the time.
- The schema gate failed on the dropped `outbox_pending_ix`, and the corpus gate
  refused the new 654,000-row table until it was registered as tenant-scale with
  a probe covering it.
- The `EXPLAIN` gate preferred a different index over the first version of
  `outbox_delivery_pending_ix`. Leading it with `consumer` fixed that, and the
  reason is now in the migration: a worker polls for exactly one consumer, so a
  time-leading index makes it walk five others' due rows to reach its own.

The dead-letter design item RB-01 in [50](50-RUNBOOKS.md) raised — dead rows
being the oldest pending rows and so sitting at the head of the poll index — is
**closed**, by the partial index excluding them rather than by a rule asking the
query to.

Still missing before `Gated`: the dispatcher worker itself and its bypass role
(the runtime half), the at-least-once acceptance test named in
[25](25-EVENTS-OUTBOX-AND-AUDIT.md), and the 7-day cleanup sweep. C-011 is
`Building`, not `Built`.

**C-003 is `Building`.** The resolution core is implemented in
`casual-task-authz` — the scope containment chain, the additive union, the
closed five-constraint set, `allows`, and `explain` — with 17 tests and no
database, which is what [19](19-WORKSPACE-SCAFFOLD-DESIGN.md) isolates that
crate for. It did **not** depend on D-032: the resolver takes an
already-authenticated actor, so the auth mechanism could be settled separately —
and was. Still missing before it can be `Gated`: the `authz_epoch` cache, the
grant and scope ceilings, and the C-004 matrix and escalation suites that are
its acceptance gates.

**C-001 is unblocked.** ADR-032 is Accepted, which is what this document's
`Accepted` requires — "design final **and** its ADRs Accepted". It carries two
schema changes into C-001's first migration: `api_token.token_hash` becomes
`token_selector` + `verifier_hash`, and the auth-storage tables are added with a
written exemption in migration 0010's block. `principal_type` is deliberately
**unchanged**.

## Phases 2–4

Rolled up until Phase 1 closes; expanded at each phase gate.

| ID | Item | Status |
| --- | --- | --- |
| A-001…A-0xx | Custom roles, simulator, workflow editor + status migration, environments, milestones, dependencies, audit console, SSO, admin console, bulk ops | Accepted (design), not scheduled |
| P-001…P-0xx | Declarative plugins, remote HTTPS, sandboxed frontend, SDK | Accepted (design), not scheduled |
| P-000 | **Three paper integrations against the extension points** | Accepted — **precondition for P-001** |
| V-001…V-0xx | Automation engine, reporting, calendar/timeline plugins, SCIM | Accepted (design), not scheduled |

## Current state

**Phase 0 — foundation. Closed 2026-08-08.** Phase 1 is open.

### The exit gate, checked

[06](06-ROADMAP-AND-DELIVERY.md) §Phase 0 states five conditions. Each was
verified by running it, not by reading the row:

| Condition | Evidence |
| --- | --- |
| CI green on an empty workspace | 12 jobs; fmt, clippy `-D warnings`, architecture lints, tests, schema, `explain-no-seq-scan`, bundle-size, docs, dependency-policy, documentation, image, platform matrix |
| Seed generates the reference corpus | 2,000,000 tasks / 38,981,941 rows / 10.2 GiB in 18 s at a 26 MiB peak RSS, byte-identical across runs, loaded into PostgreSQL 16 |
| The load-test harness runs and reports | Ran end-to-end at reference scale, and again on a quiet machine to produce a committed measurement |
| The measured bundle floor is recorded | 113.2 KiB gzip against 200 KiB. It did **not** exceed, so no superseding ADR was required; ADR-024 now records the outcome and the unit |
| The threat model is signed off | Reviewed and recorded in [07](07-QUALITY-SECURITY-AND-COMPATIBILITY.md) §Review with reviewer and date. **Read the caveat there:** the review was conducted by an agent and says so, and asks to be countersigned by a human before the Phase 1 gate. Phase 0 was closed on the project owner's direction with that caveat standing |

### What "closed" means here, and what it does not

Eleven of the sixteen Phase 0 rows are `Gated` — each verified to map to a CI job
that actually runs its harness, rather than to a row that claims one. Five are
`Built`, each with the reason it is not `Gated` written down: the latency gate
has no reference machine to produce a comparable baseline on (F-007), the
observability skeleton has no exporter (F-009), and governance files, runbooks
and a threat model are prose that no gate beyond link resolution can hold
(F-011, F-013, F-014).

**No product functionality exists.** Phase 0 built none, by design. What it
built is the ability to tell when a later phase is wrong.

### Decisions

Thirty-four `D-###` rows are `Designed` — the documentation phase, complete.
Ten carry an explicit `Accepted` decision, and **all ten were settled on
2026-08-08**: the auth mechanism, the MSRV policy, and the eight opened by
auditing Phase 0's own work. **D-033**, **D-034** and **D-045** are deliberately
deferred with reasons.
**D-048** (pin base images by digest rather than tag) is open and due before the
first release — it was opened by the threat-model review, which found that
"pinned base images" described mutable tags.

Eight of the decisions accepted on 2026-08-08 have not yet had their design
notes rewritten; the note above §Phase 0 says which, and flags the one that is
actively misleading until it is.

