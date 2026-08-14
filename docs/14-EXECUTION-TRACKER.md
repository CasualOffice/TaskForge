# 14 — Execution Tracker

Live state of all work. Every non-trivial change gets a row before it is built
([11](11-DESIGN-FIRST-PROCESS.md)), and the row moves as it progresses.

**Last updated: 2026-08-09.**

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
| D-046 | **Outbound mail security: STARTTLS requirement and certificate verification** | [29](29-NOTIFICATIONS-AND-DELIVERY.md) | **Consumed** — STARTTLS + certificate/hostname verification, in `casual-task-infra::mail` with C-001's reset endpoints |
| D-047 | **What `outbox_lag_seconds` measures, and how a cache-hit ratio is exported** | [46](46-OBSERVABILITY-AND-OPERATIONS.md) | **Consumed** — outbox lag is the age of the oldest actionable pending delivery; the authorization cache exports cumulative process-lifetime hits divided by all lookups, zero before the first lookup. |
| D-048 | Pin base images by digest rather than tag | [07](07-QUALITY-SECURITY-AND-COMPATIBILITY.md) | **Blocked** — Accept before the first release |
| D-049 | Is assigning a role distinct from authoring one at workspace scope? | [04](04-RBAC-AND-AUTHORIZATION.md) | **Consumed** — yes, distinct: `role.assign` vs `role.manage`, migration 0015 |
| D-053 | A closed event-type registry, as the permission set has | [25](25-EVENTS-OUTBOX-AND-AUDIT.md) | **Open** — surfaced by F-009 |
| D-052 | Whether a shared test-support crate should exist | [19](19-WORKSPACE-SCAFFOLD-DESIGN.md) | **Open** — surfaced by C-011 |
| D-050 | Database TLS, and the `CDLA-Permissive-2.0` licence it requires | [52](52-DEPLOYMENT-GUIDE.md) | **Consumed** — no database TLS; trusted network required, and the licence gate is what holds it |
| D-058 | `conflicting_fields` / `your_safe_fields` in the 409 body | [24](24-CONCURRENCY-AND-IDEMPOTENCY.md) | **Open** — surfaced by C-006. Accept before C-018's optimistic UI. (Was numbered D-055 by one branch while another used that number for the error-code drift; renumbered on integration.) |
| D-051 | How `key` (`WR-125`) is filtered, given it spans two tables | [27](27-FILTER-AND-SAVED-VIEW-DSL.md) | **Blocked** — was due before C-013 and is still open; C-013 ships the grammar with `key` compiling to `FALSE` and says so |
| D-054 | **How a workspace acquires its first grant** | [04](04-RBAC-AND-AUTHORIZATION.md) | **Accepted** — `docs/04`'s five templates are materialized per workspace and its creator is granted `Owner` at `WORKSPACE` scope, in the creating transaction |
| D-055 | Four shipped error codes are not in the registry (`TF-REQ-*`, `TF-SRV-*`) | [20](20-ERROR-CODE-REGISTRY.md) | **Consumed** — retired in favour of registry codes; the gate is now total |
| D-056 | The template permission sets `docs/04`'s prose does not decide | [04](04-RBAC-AND-AUTHORIZATION.md) | **Open** — surfaced by D-054. Accept with C-004's golden matrix |
| D-057 | Which permission governs workspace membership, team management **and invitations** | [04](04-RBAC-AND-AUTHORIZATION.md) | **Open** — surfaced by C-002, widened by C-001's invitations; Accept before C-002 is `Gated` |
| D-059 | Notification preferences, subscriptions, quiet hours and digests — the tables `docs/29` assumes and the schema does not have | [29](29-NOTIFICATIONS-AND-DELIVERY.md) | **Open** — surfaced by C-016. Accept before C-016 is `Gated` |
| D-060 | How the worker obtains a `BYPASSRLS` DSN, given `docs/48` names one `DATABASE_URL` | [48](48-DEPLOYMENT-PROFILES.md) | **Consumed** — `DISPATCHER_DATABASE_URL`, a second DSN as `taskforge_dispatcher`. It was already in `deploy/docker-compose.yml` and read by nothing; now documented in [48](48-DEPLOYMENT-PROFILES.md) and wired into both binaries |
| D-061 | **What a board column is: a permanent state or a workflow status** | [23](23-WORKFLOW-AND-STATE-MACHINE.md), [42](42-FRONTEND-ARCHITECTURE.md) | **Open** — surfaced by C-018. Shipped as the five permanent states, with the cost stated; see below |
| D-062 | **What a deployment with no malware scanner does with an attachment** | [28](28-ATTACHMENT-PIPELINE.md), [24](24-CONCURRENCY-AND-IDEMPOTENCY.md) | **Accepted — fail closed. Countersigned by the project owner on 2026-08-10.** Implemented as: no scanner ⇒ the row stays `PENDING` ⇒ it is never downloadable. The opposite default is a silent lie, so it is not one an implementation may pick alone |
| D-063 | **Time tracking: whether it exists, and in what shape** | [12](12-COMPETITIVE-ANALYSIS.md), [13](13-PARITY-CHECKLIST.md) | **Open** — surfaced by the parity review. It is on the category baseline [12](12-COMPETITIVE-ANALYSIS.md) names, and is in neither [01](01-ORD.md)'s FR list nor its non-goals. A duration on a task or timed entries; who may see whose time; whether it feeds `cycle_time`, which [38](38-REPORTING-EXPORT-AND-DASHBOARDS.md) currently derives from state intervals. Coding it first would settle all of that by accident |
| D-064 | **How long an MFA step-up lasts** | [40](40-IDENTITY-AUTH-AND-SESSION.md) | **Open** — surfaced by C-001's MFA. `docs/40` says a workspace "demanding more than the session carries triggers a step-up" and sets no lifetime, so none is applied: a session that has stepped up stays satisfied until it ends. `session.mfa_satisfied_at` records the instant, so a lifetime is a comparison in one function with no migration and no client change. Accept before enforcement is offered to customers. (Asked for as D-056, which C-004 had already taken; then D-059, which C-016 took; then D-062, which C-010 took. Renumbered on integration each time.) |
| D-069 | **How the declared trigram path is wired, given D-043** | [26](26-SEARCH-INDEXING-AND-QUERY.md) | **Accepted 2026-08-13, in two parts.** The finding stands: `migrations/0009_search.sql` creates `task_search_trgm` and `search::refresh` fills `title_trgm` on every write, and nothing reads either; `backu` and `bakcup` both found nothing. **Part one, built (C-050):** prefix is served by a `:*` on the final token through `to_tsquery`, which the existing `task_search_gin` already answers and which leaves the plan shape alone — chosen over the `OR title_trgm % $3` the doc implies because `compile_search` records **D-043**, that `@@` is a non-`LEAKPROOF` `ts_match_vq` under row-level security, so an `OR` across two indexes is a plan change to be measured rather than reasoned about. Measured either way: the `explain-no-seq-scan` gate returns 29 index-served and 0 sequential scans both before and after, with an identical advisory list, so the prefix form regressed nothing. **Part two, open:** typo tolerance (`bakcup`) still needs the trigram `OR`, and it lands only behind an EXPLAIN run at reference scale. If it cannot be served without a sequential scan, the index and `title_trgm` are dropped rather than left written-and-unread |
| D-070 | **The recency-decay curve, and what it does to the rank cursor** | [26](26-SEARCH-INDEXING-AND-QUERY.md) | **Accepted 2026-08-13, not yet built.** [26](26-SEARCH-INDEXING-AND-QUERY.md) §Weighting specifies "`ts_rank_cd`, with a recency decay"; `compile.rs`'s `RANK` is bare `ts_rank_cd(s.document, q)`. The decay is computed against a **reference instant captured on the first request and carried in the cursor**, not against `now()`. The reason is the half that makes this more than a one-line change: `RANK` is simultaneously the `ORDER BY` expression and the keyset cursor's sort key, so a rank that moves with wall-clock time makes page two disagree with page one — rows skipped or repeated, on the second page only, which is where a first-page test never looks. Quantising to a day boundary was the cheaper option and was rejected: it trades a correctness guarantee for a midnight edge case, in the one place a bug is invisible. Cost accepted: the cursor gains a field, and every cursor issued before it must still parse. The curve and its half-life are settled with the implementation |
| D-067 | **Whether a workspace may define its own task types** | [23](23-WORKFLOW-AND-STATE-MACHINE.md), [27](27-FILTER-AND-SAVED-VIEW-DSL.md) | **Open** — asked for directly. `task_type` is a PostgreSQL enum (migration 0001) read by the filter grammar, the `task_type_in` permission constraint (C-025), the create menu and every report dimension. Making it workspace-defined is a schema change plus a decision about what a *closed* set buys: today a filter naming an unknown type is refused at the edge rather than returning nothing, and a grant may name the types its holder may raise. Both properties come from the set being finite and shared. Accept before building — the migration is the cheap part and the grammar is not |
| D-068 | **Whether SMTP is per-workspace or per-deployment** | [29](29-NOTIFICATIONS-AND-DELIVERY.md), [48](48-DEPLOYMENT-PROFILES.md) | **Open** — asked for directly. `TF_SMTP_*` is deployment configuration today ([48](48-DEPLOYMENT-PROFILES.md)), so one relay serves every tenant. A settings screen means relay credentials stored per workspace, which brings three questions the code cannot answer alone: where the password lives at rest and under which key; whether a tenant may send as any `FROM` domain it likes, which is a spoofing and deliverability decision, not a form field; and what happens to queued mail when a workspace's relay is wrong. Accept before a settings screen is drawn |
| D-066 | **Whether a workflow belongs to a project or to a workspace** | [23](23-WORKFLOW-AND-STATE-MACHINE.md), [22](22-DATABASE-SCHEMA.md) | **Open** — surfaced by C-036. `project.workflow_id` is a real column and `ProjectView` has carried it since 0004, which reads as per-project workflows; but `ensure_default_workflow` hands every project in a workspace the *same* default and nothing creates a second one. So renaming a status renames a column on every board in the workspace, and the schema says otherwise. `/settings/workflow` states the shared reality rather than offering a project picker that would imply a choice the product does not have. Accept before a customer authors a second workflow |
| D-065 | **A time-zone database, so an offset can be derived without a client** | [27](27-FILTER-AND-SAVED-VIEW-DSL.md), [40](40-IDENTITY-AUTH-AND-SESSION.md) | **Open** — `user_account.time_zone` stores an IANA name (migration 0030) and evaluation uses the offset the client sends, which a browser computes correctly including daylight saving. A server-side job has no client to ask, so digests and scheduled notifications cannot resolve `@today` for a user until a tz database is a dependency |

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
| F-009 | Observability skeleton | **Gated** | F-001 |
| F-010 | Docker Compose dev profile | **Gated** | — |
| F-011 | Governance files, Apache-2.0, AGENTS.md | `Built` | — |
| F-012 | **Bundle floor measurement** (ADR-024) | **Gated** | — |
| F-013 | Threat model review | `Built` | — |
| F-014 | Runbooks (initial set) | **Gated** | F-009 |
| F-015 | Migrations + application role + schema verification gate | **Gated** | F-005 |
| F-016 | Container image, deployment compose, deployment guide | **Gated** | F-015 |

## Phase 1 — Core (C)

Phase 1 closure follows [54](54-PHASE-1-CLOSURE.md): C-001–C-005 authority and
isolation first, then C-007–C-011 command integrity, C-012–C-016 query and event
contracts, and C-017–C-021 client and edge closure. No new Phase 2–4 capability
lands in core until these rows are `Gated` or carry a named decision or external
measurement blocker.

| ID | Item | Status |
| --- | --- | --- |
| C-001 | Identity, sessions, MFA, invitations | `Built` |
| C-002 | Workspace, membership, teams | `Built` |
| C-003 | **Permission resolver + `/explain`** | `Built` |
| C-004 | Permission matrix + escalation suites | `Built` |
| C-005 | Cross-tenant property suite | `Gated` |
| C-006 | Projects, membership, visibility | `Gated` |
| C-007 | Default workflow + transitions | `Building` |
| C-008 | Task CRUD, assignees, tags | `Built` |
| C-009 | Comments | `Built` |
| C-010 | Attachment pipeline | `Built` |
| C-011 | Activity + audit + **outbox** | `Building` |
| C-012 | Filter grammar + compiler | `Building` |
| C-013 | Search projection + full-text | `Built` |
| C-014 | Cursor pagination | `Built` |
| C-015 | SSE + fan-out | **Gated** |
| C-016 | Notifications (in-app + email) | `Building` |
| C-017 | Extension point registry (core panels only) | `Built` |
| C-018 | Web shell, board, list, My Work, drawer, palette | `Building` |
| C-019 | Bundle + a11y gates wired | `Built` |
| C-020 | Rate limiting at the edge | `Building` |
| C-021 | **Export** — CSV/JSONL of any task query, as a job | `Building` |
| C-022 | **Chain of custody** — team transfer, environment promotion, verification, `/me/queue` | `Built` |
| C-023 | **Releases** — what went out together, cut from the pipeline | **Gated** |
| C-024 | **Team scope** — team as a place to stand, beside project and workspace | **Gated** |
| C-025 | **Who may raise what** — `task_type_in`, decoded, enforced and offered | **Gated** |
| C-026 | **Reports** — a filter plus a grouped count (ADR-027), `count` only | **Gated** |
| C-027 | **Stylesheet gate** — one spacing scale, no duplicate rules | `Built` |
| C-028 | **Environments as configuration** — add, rename, reorder, remove | **Gated** |
| C-029 | **State-occupancy projection** — `task_state_interval`, maintained and rebuildable | **Gated** |
| C-030 | **Duration measures** — cycle time, lead time, throughput | **Gated** |
| C-031 | **Popover placement and the narrow list row** — two release blockers | `Built` |
| C-032 | **The browser layer** — geometry, reflow and touch targets, measured | `Built` |
| C-033 | **The list, to its own spec** — status, assignee, column filters, grouping | `Built` |
| C-034 | **An empty body is not a payload** — the transport refuses a silent `undefined` | **Gated** |
| C-035 | **Dashboards** — the four built-ins, five visualizations, no charting library | **Gated** |
| C-036 | **Projects, and the shell that stopped scrolling** — create and edit a project; the rail and header stay put | `Built` |
| C-037 | **The phone, to its own spec** — audit items 2, 3 and 5 closed, measured | `Built` |
| C-038 | **A workspace you can start** — audit item 10; the first run is no longer a dead end | `Built` |
| C-039 | **The create-task flow, and the workspace in the header** | `Built` |
| C-040 | **Attachments reach the browser** — the preflight that made `docs/28` usable | `Built` |
| C-041 | **The task drawer belongs to the address** — Home and Environments could not preview | `Built` |
| C-042 | **The attachment scan** — `docs/28` step 4, the consumer that made uploads visible | `Built` |
| C-043 | **The `age` measure** — how long open work has been waiting | `Built` |
| C-044 | **`created_vs_completed`, and the card that dragged behind the board** | `Built` |
| C-045 | **Reports draws the same charts the dashboard does** | `Built` |
| C-046 | **`time_in_state`** — the last measure the closed set specified | `Built` |
| C-047 | **Deployment story and the public site** — build-from-source compose, environment reference, README, GitHub Pages | `Built` |
| C-048 | **People in the palette** — the third of "tasks, projects and people" nothing fetched | `Built` |
| C-049 | **A search result that says why it matched** | `Built` |
| C-050 | **Prefix search** — a word finds its task before it is finished (D-069 part one) | `Built` |

The implementation notes below are chronological evidence: they record what a
change found, why it took its shape, and what was missing at that point. The
table above and the closure note in [54](54-PHASE-1-CLOSURE.md) are the current
status. When an older “still to come” statement conflicts with them, the table
wins; known contradictions are corrected in place rather than treated as open
work.

**C-023, C-024 and C-025 are `Gated` at both ends.** The servers are protected by
`cargo test --workspace -- --ignored`, which CI runs; the clients by
`webapp/src/requests.test.tsx`, which renders the real views against a stub
server and asserts **the request that leaves the client** — the query string a
scope produces, the body a release cut posts, the types the create menu offers.
The assertion is on the outgoing request rather than the rendered rows because
the rows come from the stub, and asserting a stub asserts nothing. Each of those
tests was checked by breaking the code it covers and watching it fail; a suite
nobody has seen fail is a suite nobody should trust.

**C-022 is still `Built`.** Its server is gated by `tests/custody.rs`; its
client — the transfer, promotion and verification forms — is not. It is the
least risky of the four (a form that posts what was typed, with no derived
scope in between), which is why it is last, not why it is fine.

**C-032 exists because three changes in a row shipped with "not visually
verified" in their descriptions.** jsdom has no layout and no painting, so a
popover positioned off-screen, a metadata rail stacked above the title it
belonged under, and a list row whose title collapsed to nothing all passed
`tsc`, `eslint` and the axe suite on their way out. The browser layer asserts
geometry: what is on the screen, how wide it is, whether anything overflows, and
how big a control is under a finger.

It found three of the audit's open items on its first run, and they are recorded
as **expected failures** rather than skipped — the assertion still runs, and the
day the layout is fixed the suite reports an *unexpected pass* and forces the
marker to be deleted. A skipped test is a test nobody removes.

- ~~**item 2** — the shell is wider than a 390 px viewport on several routes~~ — closed by C-037
- ~~**item 3** — the stacked list row does not hold at 390 px~~ — closed by C-037
- ~~**item 5** — controls under the 44 px tier~~ — closed by C-037

**C-037 closed items 2, 3 and 5, and every marker is gone from the suite.** That is
the mechanism working as designed: the fix made nine assertions report an
*unexpected pass*, which is the only signal that reliably gets a stale
expectation deleted.

**Item 2 was three fixed widths.** The header carried `min-width: 260px` on the
search and rendered the theme state as 113 px of visible words, and the bottom
bar sized eight destinations by their labels — "Environments" alone is 84 px.
583 px of shell in a 390 px window. The search keeps its icon and loses its
floor; the theme toggle keeps its icon and loses its words, which its
`aria-label` was already carrying; the bar shows icons, with the labels
**clipped rather than removed** — `display: none` would strip each link's
accessible name and leave a screen-reader user with eight unnamed buttons,
which is a worse bug than the one being fixed.

Eight is still more than the four or five a phone tab bar conventionally
carries. Trimming to five with the rest behind a "More" sheet is a product
decision about which destinations are primary, so it is recorded rather than
guessed at.

**Item 3 had two causes, and the second one was mine.** The narrow row placed
cells with `nth-child(4)`, `(5)` and `(6)`; C-033 then added Status and Assignee
columns, which shifted every one of those selectors two places to the right.
Due and Updated had no area left, so the grid auto-placed them on top of the
title. Cells are placed by **name** now: a positional selector is a rule that
breaks when a *sibling* is added, which is exactly the edit nobody thinks to
re-check.

Underneath that, every virtualized row was given a fixed `height` from a single
`ROW_HEIGHT = 40` constant — true of the desktop table, false of a four-line
stacked summary whose height depends on how far the title wraps. Rows now
report their real height through `measureElement`.

**The suite passed through all of it.** Three width assertions were green while
the row rendered as four lines of text on top of each other, because every one
of them measured *widths*. `a list row contains its own cells` is the assertion
that was missing — it fails with `title spans 16..58 of a 40px row`, and it was
verified against the pre-fix code.

**Item 5 was the design system sizing for a mouse.** Sixteen controls on the
list route and twelve in settings were under the tier — 34 px buttons, 34 px
selects, 23 px text inputs. The floor is `min-height` under `pointer: coarse`,
and both halves of that are load-bearing:

- **`min-height`, not `height`.** `@schnsrw/design-system` sets `height` as an
  *inline style*, which no selector can outrank. It does not need to be
  outranked: the used height is `max(min-height, height)` whatever the
  specificity of either, so the floor applies with no `!important` anywhere and
  without clobbering the deliberate inline tints those components also set.
- **`pointer: coarse`, not a width.** A 44 px toolbar on a desk wastes a third
  of its height. The question that matters is whether the primary input is a
  finger, and deriving that from a viewport width gets a touchscreen laptop
  wrong in both directions.

The two links inside a list row needed the floor restored explicitly, because
the narrow layout deliberately removes it: the title sets `min-height: 0` so it
can wrap as prose rather than sit in a 44 px line box, and the key link was
43 px wide — one pixel under the tier is still under it.

**C-034 is a fix to the fix that caused it.** `POST /teams/{id}/members` answers
`201` with no body, so the transport was taught to tolerate an empty body on any
status rather than only `204`. That was right for writes and wrong for reads:
`request<T>` and `requestWithVersion<T>` then returned `undefined` under a type
that promised `T`. Nothing threw, so the query settled as a **success** carrying
no payload, every `if (query.error)` in the app passed it through, and the fault
detonated one component later — `/settings/workflow` reported `Cannot read
properties of undefined (reading 'id')` from inside `Statuses`, with no error
code, no `request_id`, and nothing tying it to the response that caused it. Two
screens unwrapped a payload that way (`workflow`, `workspace`) and both would
have needed their own guard, forever, on every screen added after them.

The fix is at the boundary rather than at the call sites: **"this route returns
nothing" is now a claim a route makes once**, by calling `requestNoContent`
instead of `request<void>` — nine routes do — and anything else that arrives
empty raises `TF-SYS-0001` naming the path. So the two screens did not need
guards at all; the state they were guarding against can no longer be
constructed. Registry note: `TF-SYS-0001` rather than a new code, because
`docs/20` owns that catalogue and this is exactly the server fault the user did
not cause that the generic code already describes.

Migrating the nine caught a live bug on the way past: `logout` used
`request<unknown>` and `POST /auth/logout` answers `204`, so signing out would
have started raising. `tsc` could not see it — `unknown` accepts `undefined` —
which is the argument for the split being a *function* rather than a convention.
Gated by four cases in `webapp/src/api/http.test.ts` covering both directions,
each checked by removing the guard and watching them fail.

**C-036 closes two holes that made the product unusable without going around
it.**

**There was no way to create a project.** A project is what tasks are created
in, what the board is scoped to, what a workflow belongs to and what permissions
are granted on — and the only way to get one was a seeding script. So the first
thing a new workspace owner could do was nothing. `/settings/projects` creates
and edits them. `key` is offered only at creation and shown read-only
afterwards, because ADR-007 freezes it and the server answers `422` to any
attempt to change it: a control that exists and always fails is worse than one
never drawn. The key is uppercased as it is typed and the hint previews the task
keys it will mint (`MOB-1, MOB-2`), so the permanence is legible before the
decision rather than after it.

**`/settings/workflow` was editing a project nobody had chosen.** It took
`projects.data[0].workflow_id` and said nothing, so with five projects it edited
one picked by list order. The first fix was a project picker — and that was
wrong, because it implied a choice the product does not have: `ensure_default_workflow`
gives every project in a workspace the *same* workflow, so renaming a status
renames a column on every board. The page now states that instead, naming the
projects affected. **D-066** carries the underlying question, since
`project.workflow_id` is a real column and the schema disagrees with the
behaviour.

**One query key, two shapes.** `/settings/workflow` and the board read the same
workflow from the same URL, and cached it under the same key — but through
different functions: `readWorkflow` returns a `Workflow`, `readWorkflowForEditing`
returns `{ data, version }` because the editor needs the `ETag` to write.
Whichever query ran last won, so opening settings and then *any* board crashed
it with `workflow.statuses is not iterable`: the board had been handed the
editor's envelope.

Neither request was wrong and no type caught it — `useQuery` infers its data
type from its own `queryFn`, so both screens type-checked against a cache entry
only one of them could be right about. The rule now written into `keys.ts`:
**two reads that return different shapes are two cache entries, even for the
same resource behind the same URL.** The editor's key is a *child* of the
board's rather than a sibling, so the prefix invalidation every workflow write
already performs still repaints the board — splitting them into siblings would
have traded the crash for a staleness bug, and `keys.test.ts` fails on both
mistakes.

**The whole application scrolled.** `.shell__main` had no base rule at all — it
was `overflow: visible` and `display: block`, so content grew the grid row, grew
the document, and the sidebar slid up out of view with the header behind it on
every route long enough to scroll. Two other rules in the same stylesheet
already described `.shell__main` as `hidden`, and the narrow layout overrode it,
so the one composition nobody had written a rule for was the one everybody uses.

The gate for it injects 3000 px of content rather than waiting for a route to be
long enough on its own: with the rule deleted only *one* of five routes had
enough stub rows to overflow, so four of them would have gone on passing through
the regression. Verified by deleting the rule and watching all five fail.

**C-035 exists because the product had no dashboard at all.** Reports answered
one question at a time and had to be driven to answer it; there was nowhere to
*look* at how work was going. `docs/38` had specified the whole surface — a
dashboard is a named layout of reports, four built-ins ship, six visualizations,
closed — so this is that design executed rather than a new one.

The built-ins are **data, not components**: `builtin.ts` is `filter` + `measure`
+ `group_by` + `bucket` per tile, posted to the same endpoint a user-composed
dashboard will use. `docs/38` asks for exactly that, on the grounds that a
built-in needing a capability the model lacks is a signal the model is
under-specified — and writing them out found four such gaps (created vs
completed, age of oldest open, reopen rate, time in state). They are **absent
rather than approximated**: a wrong number on a dashboard gets quoted in a
meeting, where a missing one gets asked about.

**No charting library.** Recharts is ~95 KiB gzip against 40 KiB of headroom;
the closed visualization set is what makes drawing five shapes by hand
reasonable, and the whole route including its stylesheet is 5.1 KiB gzip in its
own chunk. Initial shell moved 159.7 → 159.9 KiB. Every chart renders its SVG
`aria-hidden` beside a visually-hidden `<table>` of the same numbers — the
drawing is decoration, the table is the content.

Three defects the work surfaced, none of which any unit test could have seen:

- **The dashboard was a load generator.** Nine tiles mounted together, sent nine
  reports at once, and a run of them came back `503`. `docs/38` caps reports at
  5 concurrent per workspace, and the edge's answer to breaching that is a
  refusal — rendered as an error in a tile whose number was perfectly
  computable. The client now queues its own tiles at 4, leaving a slot for a
  second tab. Gated by `gate.test.ts`, five cases, four of which fail with the
  bound removed.
- **A hidden table pushed the page 62 px wider than the viewport.**
  `.visually-hidden` clips with `width: 1px; overflow: hidden`, and neither
  constrains `display: table` — a table sizes to its content and does not clip.
  So each chart's data table was invisible *and* causing a horizontal
  scrollbar. Caught by the desktop half of the overflow assertion on its first
  run.
- **The dev database was three commits behind its own migrations.** `0032` had
  never been applied, so every duration measure answered `500` locally while
  passing in CI against a fresh database.

The mobile overflow assertions are marked `test.fail` for audit item 2, and that
inheritance was **measured rather than assumed**: at 390 px every overflowing
element is shell chrome — `shell__search`, `side__link`, the account popover —
and not one carries a `dash__`, `tile` or `chart__` class.


`/settings/roles` and the task detail route are **not** covered: both need a
much fuller fixture than a layout suite should carry, and a stub grown to
satisfy them is a stub nobody trusts. Their behaviour is covered by the jsdom
suite; their geometry is not.

**C-029 is `Gated` now that C-030 reads it.** The
projection exists, the outbox maintains it, and
`the_state_projection_is_rebuilt_from_history_and_survives_redelivery` pins the
property everything else rests on: delivery is at-least-once, so a consumer that
appended an interval per event would double a task's history the first time one
was redelivered — and every duration number would be quietly wrong with nothing
on screen to say so.

One deviation from `docs/38` worth recording. It says the projection is
"rebuildable from `activity_event`", and in this implementation it is not:
`docs/25` requires the activity stream to carry **display values** — the status
*names* — precisely so an entry survives a rename. The audit stream carries the
ids and the state on both sides of every transition, which is what an interval
needs, so the rebuild reads that instead. The design's intent holds; the table
it named does not.

**C-027 exists because I kept making the same two mistakes.** Three times in one
session I appended a declaration to a stylesheet that already defined the same
selector, and the later copy silently won — once changing the shape of every
settings list on every settings page. Separately, two spacing scales with
colliding indices (`--space-4` is 8 px, `--tf-space-4` is 16) were mixed 92
times to 57 in one file, so the gaps actually rendered were 1, 2, 4, 5, 6, 7, 8,
11 and 14 px and nothing looked layered.

`pnpm lint:css` now fails on structural spacing that is not a grid step or a
layout token, and reports duplicate selectors. Duplicates are **warnings, not
errors**: eighteen predate this change — `.card`, `.textarea`, `.pill`,
`.filter__option`, `.side__link`, `:root` and more — and merging eighteen rules
blind, without reliable visual verification, is how the pages got worse in the
first place. Merging them is the next job and the rule becomes fatal with it.

It has already earned itself: it found `.list__cell` setting `font-weight:
inherit` *above* a `font:` shorthand, which resets it — so a list cell never
inherited its row's weight, on the surface most complained about.

**C-026 is a Phase 4 row built in Phase 1, and that is a scheduling decision
rather than a design one.** `docs/38` places reports in Phase 4; ADR-027 already
fixes what a report *is*, so nothing here was invented — but the sequence was
brought forward because the product's stated purpose includes generating a
report, and the nav item said "not built" while claiming the capability existed.

Its measure set is **`count` alone**. `cycle_time`, `lead_time`, `time_in_state`
and `throughput` all read state occupancy, which `docs/38` maintains as the
`task_state_interval` projection that the outbox worker does not build yet.
Computing them by replaying `activity_event` per request is the unbounded query
that document exists to prevent, so the endpoint refuses them **by name** with
`TF-SYS-0007` — answering a request for `p50 cycle_time` with a count would hand
someone a figure that is wrong in a way nothing on the page reveals. The
projection, its worker consumer and its rebuild are the next row, not a
follow-up to this one.

**C-025 is `Gated`, and it is a repair.** `task_type_in` has been in the closed
constraint set since `docs/04` with its own unit tests, and `explain` has known
its name — but `constraints_of` had no arm for it, so every stored grant
carrying it decoded to *unsatisfiable*, and `create` authorized with no task
type in the facts, so even a decoded constraint matched nothing. Two independent
reasons the rule denied its holder everything, both silent, because a permission
that denies looks like a strict administrator rather than a defect. The gate is
`a_developer_may_raise_the_types_their_grant_names_and_no_others`, which asserts
the allow, the deny, the default type, the convert-after-create escape, and the
menu the client draws.

**C-022 went untracked while it was built**, which this row is correcting rather
than hiding. AGENTS.md asks for the row when the work *starts*; it was written
across several PRs that each looked like a continuation of C-018.

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

**C-005 is `Gated`.** The persistence seam is backed by the route-derived API
gate in `casual-task-api/tests/route_isolation.rs`: every route not explicitly
classified as public, pre-workspace or actor-only is driven as an authenticated
non-member and must hide the workspace with `404`. The test is Docker-backed
and runs in CI's blocking schema job with every ignored acceptance suite.
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

Status authoring, ordering, migration, workflow reads and the transition command
have since landed. C-007 remains `Building`: migrations above the synchronous
10,000-task ceiling need the tracked background path `docs/23` specifies, and
Phase 3 transition hooks remain intentionally absent until the plugin runtime
exists.

The dependency override acceptance path is now specified without adding a
second reason field: a non-empty transition `comment` is required exactly when
unresolved blockers are bypassed by `task.dependency.override`, and the same
text is written as the task comment and as `dependency_override.reason` in the
immutable audit change. The gate covers refusal without a reason, success with
one, the visible blocker ids in audit, and the fact that an
`ignore_dependencies` edge does not manufacture an override.

**C-004 is `Built`.** Five controls live in `casual-task-authz`; last-owner
protection is checked under a database lock; every grant and role edit records
audit history through the unit of work. The API escalation suite attempts
controls 1–5 and 7 end to end, while the plugin-ceiling test attempts control 6
at the contract boundary that exists before Phase 3 supplies an installation
route.

Both property tests docs/04 §Acceptance gates names are in: additivity — "the
invariant the whole model rests on" — and cross-workspace isolation. Both are
seeded so a failure names its reproducing case, and both were verified to fail:
injecting a most-specific-wins rule between grants makes additivity red at a
named seed. A third test asserts the generator produces both allows and denies,
because a property test over a generator that never allows anything is
vacuously true.

Still missing before `Gated`: D-056 must settle the golden built-in-role matrix.
The full-page one-resolution acceptance test and the route-derived 404 sweep now
run in the blocking schema job.

**Sessions are now consumed, not just created.** Two extractors, and the split
between them is the design `docs/40` §Workspace-level SSO and MFA step-up
describes: "signed in" and "may enter this workspace" are different questions,
because the browser session is user-scoped while membership and MFA policy are
per workspace.

- `Authenticated` — a live session cookie or bearer token. Knows *who*, not
  *where*.
- `WorkspaceMember` — the above plus a workspace validated against membership on
  **this** request. It is the only thing in the codebase that mints an
  `AuthContext`, which is what makes `WorkspaceScope` unforgeable elsewhere
  ([32](32-TENANCY-AND-ISOLATION.md)).

A handler taking `Authenticated` **cannot reach tenant data**, because it has no
`AuthContext` to build a scope from. That is a compile-time property rather than
a review note, which is the shape [10](10-PROJECT-GOAL-AND-STANDARDS.md) §3 asks
for.

**A workspace an actor is not a member of is 404, not 403** — including a
workspace that genuinely exists. [04](04-RBAC-AND-AUTHORIZATION.md) requires
absent and invisible to be indistinguishable, and a 403 would let an
authenticated stranger enumerate workspace ids by probing a header. The test
compares a real workspace against an imaginary one and asserts the responses
match.

**CSRF is enforced as a layer over every route**, not per handler. `docs/05` says
"every unsafe method without a valid token is rejected", and *every* means the
guard has to sit somewhere a new route cannot be added beside. It sits **under**
the observability layer, so a CSRF rejection still gets a request id and still
counts in the metrics — a refusal nobody can measure is a refusal nobody
notices. Bearer-authenticated requests are exempt: a token is not sent
automatically by a browser, so requiring a CSRF token from a service account
would be asking a machine to defend against an attack that needs a browser.

**The guard broke an existing test the moment it landed**, which is the guard
working: `logging_out_revokes_the_session_immediately` posted with only a
session cookie — exactly the shape of a cross-site form submission — and now
gets a 403. It carries the token.

Bearer tokens authenticate through the ADR-032 pre-workspace seam, so C-001's
credential layer and its runtime half now meet.

**Login works.** `POST /api/v1/auth/login` and `/auth/logout`, with the session
cookie `docs/40` specifies flag for flag, and seven end-to-end tests against a
real PostgreSQL.

**The enumeration gate is met, and it is met structurally.** `docs/40` calls
account enumeration through login "the most commonly shipped auth bug" and
requires responses indistinguishable "in body, status, and timing envelope".
Two things make that hold rather than aspire:

- `LoginOutcome` has **one** failure variant. A type with `NoSuchAccount` beside
  `WrongPassword` is an oracle waiting for someone to map them onto different
  responses; there is no second value to return.
- An unknown address still performs an Argon2 verification against a fixed dummy
  hash. Skipping it returns in microseconds against ~100 ms for a real account —
  an oracle wide enough to read with a stopwatch. The test asserts the two paths
  stay within an order of magnitude, which is the strongest bound a shared CI
  runner can hold without becoming flaky, and it would catch a ratio of
  thousands easily.

**The Argon2 parameters were wrong and are now right.** The code used
`Argon2::default()` — 19 MiB, t=2, p=1 — where `docs/40` §Local authentication
specifies **64 MB, t=3, p=4**. Memory cost is the parameter that makes GPU and
ASIC attacks expensive, so the difference was roughly a threefold discount to an
attacker holding a dump. A test now pins all three, and another proves a hash
made with the old parameters still verifies — which is why they live in the PHC
string.

**CSRF is bound to the session, not merely double-submitted.** ADR-032 says
`TF_SECRET_KEY` "is not a cookie signature"; it is used here and nowhere else.
A plain double-submit compares two values the *client* sent, so anyone able to
set a cookie on the victim's domain sets both halves and passes. The token is
`HMAC-SHA256(key, session_selector)`, so forging one needs the selector and the
server key — and it needs no storage and no expiry of its own, because it dies
exactly when the session does.

**A test disagreed with the code, and the code was right.** Five failed logins
recorded four failures, not five: once the backoff starts, further attempts are
refused *without* counting. That is deliberate — counting them would let anyone
hold a stranger's account locked indefinitely by guessing at it forever, which
is the denial of service `docs/40` rules out. The assertion now states that
property instead of the number.

The architecture lint refused the test SQL, as it did for C-011 and the server
foundation. The fixtures moved into `casual-task-persistence`'s `test_support`
rather than the rule being bent a third time.

Still missing before C-001 is `Built`: the auth **middleware** that turns a
session into an `AuthContext` (nothing consumes a session yet — the endpoints
create and destroy them), CSRF enforcement wired as a layer, MFA step-up at
workspace resolution, invitations, and password reset. SSO is Phase 2.

**The HTTP server exists, and F-009's `/metrics` endpoint with it.** The API
binary starts, and every step of its startup is a refusal rather than a warning:
configuration that fails fast and names the variable, a bounded pool (D-039),
and the superuser check [48](48-DEPLOYMENT-PROFILES.md) requires — "the API
refuses to start if `current_setting('is_superuser')` is on". Connected as a
superuser the application works perfectly, every test passes, and tenant
isolation and audit immutability are both silently inert; there is no symptom
until one customer sees another's tasks.

`--health-check` is now real. It was a scaffold that refused rather than lied,
which was the right holding position; `deploy/docker-compose.yml` has been
probing liveness with it since F-016.

**Health is two questions.** `/health/live` touches nothing — a liveness probe
that checked the database would restart every API instance during a database
outage, removing the only thing that could still serve anything and adding a
reconnect storm. `/health/ready` does check it, and answers **503**, not 500:
"do not send me traffic" rather than "I am broken".

**The cardinality guard held at the HTTP boundary, and it mattered here.** The
route label is the router's *template*, and it is interned against a fixed table
before becoming a metric label — the request path is attacker-controlled, so
without that every 404 to a random URL would permanently add a time series. A
test fires `/../../etc/passwd` and `/wp-admin` at the server and asserts neither
reaches the scrape body.

The architecture lint refused `SELECT 1` in the readiness handler and the
superuser check in `main`. Both moved to `casual-task-persistence`
(`health::ping`, `health::is_superuser`) rather than the rule being bent for two
one-line queries — the same call made for the C-011 acceptance gate.

Config additions are documented in [48](48-DEPLOYMENT-PROFILES.md):
`TF_DB_MAX_CONNECTIONS` and `TF_DB_ACQUIRE_TIMEOUT_SECONDS`. The acquire timeout
is what makes D-039's 503 reachable; without it a saturated pool is a hang.

**C-001's credential layer is in; C-001 is `Building`.** Migration 0016 adds
`user_credential`, `session`, `mfa_factor`, `recovery_code`,
`password_reset_token` and `invitation`, and `casual-task-identity` implements
the primitives that guard them.

**The selector/verifier split, as ADR-032 settled it.** A presented credential is
`<selector>.<verifier>`: the selector is non-secret and uniquely indexed, so the
row is found in one index read; the verifier is 192 bits stored only as a
per-row salted hash. The rejected alternatives are recorded where the code is,
not only in the ADR — a single hashed column forces a choice between an
unfindable row and an unsalted hash, and a server-held pepper makes one secret
outside the database load-bearing for every authentication.

**Two hash functions, on purpose.** Argon2id on passwords and recovery codes —
low-entropy secrets a human chose or typed, where a slow KDF is the only thing
between a dump and an offline attack. SHA-256 on verifiers, which are 192-bit
random values where a slow hash buys nothing and costs latency on every
authenticated request. That argument rests entirely on the verifier length, so
it is a **compile-time** assertion rather than a test that could be deleted.

**The lockout is a counter and a time, never a flag.** `docs/40` requires backoff
"without locking a legitimate user out permanently", and a boolean `locked`
column is a denial of service anyone can trigger by typing a stranger's email
wrongly enough times. The ladder is capped at fifteen minutes — an uncapped one
reaches "locked until next Tuesday" after about twenty attempts, which is the
permanent lockout under another name.

**`Totp::verify` returns the time step, not a bool.** A code is valid for a whole
30-second window, so RFC 6238 §5.2 requires refusing a step already accepted;
that check needs storage and cannot live in the crate. Returning the step is
what keeps replay protection addable — a `bool` would have made it impossible to
add later without changing every caller.

**The pre-workspace seam is built and asserted.** ADR-032 called it "a deliberate
hole in the ADR-020 backstop … security-critical logic in SQL, outside the type
system", and made three things non-optional. All three are present:

- The pinned `search_path`, asserted by the schema gate.
- The fixed projection, asserted by the schema gate — **the gate fails if
  `verifier_hash` ever appears in `lookup_api_token`**. The hash has its own
  function precisely so no one concludes it would be convenient to add.
- Zero rows for a revoked or expired credential, asserted by eight integration
  tests running as `taskforge_app`. As the owner, RLS is inert and every one of
  them would pass without the function existing at all.

Both schema assertions were verified by breaking them deliberately: widening the
projection and removing the pinned `search_path` each fail with the message that
names the risk.

The HTTP surface — login, logout, session cookies, CSRF, MFA enrolment and
invitations — has since landed. C-001 is `Built`; the remaining move to `Gated`
is the complete acceptance contract in [40](40-IDENTITY-AUTH-AND-SESSION.md).
SSO remains **Phase 2**.

**C-002 is `Built`.** Eleven routes — create, list, read and rename a workspace;
list, add and remove its members; list and create its teams; add and remove team
members — plus the migration that makes two of them expressible. This is the row
that unblocks the product: before it, a signed-in user had no workspace, and
every other endpoint in Phase 1 is inside one.

**The membership check was returning `false` for everyone in production.**
`workspace_membership` carries `workspace_id`, so migration 0010 gave it a
policy; the check that mints an `AuthContext` runs before any workspace is set,
because that check is what decides the value. Read directly as `taskforge_app`,
the policy hid every row. Every test passed, because a test harness connects as
the database owner and RLS is inert for a superuser. Migration 0019 gives it the
ADR-032 treatment the credential lookup already had — `SECURITY DEFINER`, pinned
`search_path`, `EXECUTE` to `taskforge_app` alone, definition asserted by the
F-015 gate — and `tests/workspace_seam.rs` reproduces the original failure as
the application role so it cannot come back quietly.

**`workspace` and `team` had no `version` column**, so the two aggregates C-002
makes mutable were the two that could not express
[24](24-CONCURRENCY-AND-IDEMPOTENCY.md)'s "every mutable aggregate carries
`version`" or [05](05-API-SPEC.md)'s `If-Match`. Added in the same migration
rather than when a rename first needs it, because the direction is one-way:
shipping `PATCH /workspaces/{id}` unconditional and requiring `If-Match` later
is a breaking API change. A schema assertion now lists the seven aggregates that
must carry it.

**Tenant predicates are written out, not left to the policy.**
[32](32-TENANCY-AND-ISOLATION.md) requires two independent mechanisms that must
both fail before data crosses a boundary. The first version of the team read
carried only `WHERE id = $1` and leaned on RLS — and the cross-tenant test
caught it by returning `201` where it expected `404`, precisely because the
harness runs as the owner. Every scoped statement now names `workspace_id`, and
the two `team_membership` writes select through `team` so a team id from another
tenant affects zero rows however it was obtained.

**What C-002 does NOT do, stated plainly: membership is the only authority it
enforces.** Any member of a workspace can rename it, add and remove members, and
create teams. [04](04-RBAC-AND-AUTHORIZATION.md) gives Member "no config", so
that is not the end state — but `role_assignment` is the only source of
authority (migration 0003), no built-in role template has been authored, and the
golden matrix that fixes each template's permission set is the C-004 work listed
above as still missing. Inventing a mapping here would settle it in an
implementation, which AGENTS.md forbids. **D-054** is the open question, and it
has two halves: which permission governs membership and team management — the
closed registry has `workspace.manage` and no `workspace.member.manage` — and
whether workspace creation should seed the built-in role templates and grant the
creator Owner.

Two rules that *are* enforced, because both are decidable without a grant. A
workspace cannot lose its last member: nothing can see a memberless workspace,
so nothing can add a member back to it, and the check is made under the
workspace row's write lock so two concurrent removals cannot each believe they
are not the last. And a user added to a team must already be a member of the
workspace — `team_membership` has no policy of its own, so that check is its
tenant boundary.

**Still to come before C-002 is `Gated`: D-057.** The cross-tenant suite is now
generated from the production route table and the visibility sweep runs in the
blocking Docker job. Those gates no longer block this row.

**Idempotency is now consumed by the create paths that require it.** Migration
0008's claim/replay store is live, conflicting reuse is refused, and the API
tests exercise the replay contract rather than leaving the table dormant.

**D-055 is consumed.** Four codes this API emitted — `TF-REQ-0001`,
`TF-REQ-0004`, `TF-SRV-0001`, `TF-SRV-0003` — were not in
[20](20-ERROR-CODE-REGISTRY.md), which has no `REQ` or `SRV` area at all, so the
`docs` URL in those error bodies pointed at nothing. C-002 added the gate that
found them and named them as exceptions; the exceptions are now gone:

| was | is | why |
| --- | --- | --- |
| `TF-REQ-0001` | *removed* | unused, and a duplicate of `TF-VAL-0001` |
| `TF-REQ-0004` | `TF-AZN-0008` | a new registry row: the generic form of `TF-PRJ-0001`/`TF-TSK-0001`, in `AZN` because it is a **visibility** answer |
| `TF-SRV-0001` | `TF-SYS-0001` | "Internal error", already in the registry |
| `TF-SRV-0003` | `TF-SYS-0002` | "Service temporarily unavailable", already in the registry |
| `TF-AUT-0002` on every 403 | `TF-AUT-0008` / `TF-AUT-0013` | `-0002` is "Session expired". A CSRF failure and a wrong credential type are different answers leading to different actions, and one code sent both to the wrong page |

Renaming a code is a public-contract change ([20](20-ERROR-CODE-REGISTRY.md)
§Rules: append-only) — which is why this was its own change and not a line in
C-002. It is safe here for a reason that will not be true again: **none of the
four has ever been released.** After the first release the same drift would have
to be carried, not corrected.

The exception list is deleted rather than emptied, so the next code cannot be
added to it, and two assertions were added beside the containment check: the
gate is shown failing against a retired code, and every emitted code's **area**
must be one of the fourteen the registry declares — `TF-XYZ-0001` would pass a
substring check if the string happened to appear in prose, which is exactly the
shape of the bug being fixed.

**Password reset is in, and D-046 is `Consumed` with it.** A user who forgot
their password had no way back into the product; `POST
/api/v1/auth/password-reset` and `.../confirm` are that way, built to the four
words [40](40-IDENTITY-AUTH-AND-SESSION.md) §Local authentication spends on
them: "single-use, 1 h, hashed at rest, invalidated by password change".

**Delivery is off the request path, and that is the enumeration control rather
than a performance one.** [40](40-IDENTITY-AUTH-AND-SESSION.md) §Acceptance
gates requires reset responses to be indistinguishable for existing and
non-existing accounts "in body, status, and timing envelope". Body and status
are one constant; the timing half is what an inline send destroys, because an
SMTP handshake is tens to hundreds of milliseconds and the unknown-address
branch skips it entirely. That gap is readable with a stopwatch and is a
complete account oracle. The handler mints, stores and answers; a spawned task
talks to the relay. The cost is stated: a delivery failure reaches the log, not
the caller — which is correct here, since the caller must not learn whether an
address was deliverable either.

**Single use is a `WHERE` clause, not a read followed by a write.**
`consume_reset_token` updates only a row that is still unused and reports
whether *this* call was the one that burned it, so two concurrent confirmations
both find a live token and exactly one proceeds. Checking first and updating
second is the same code with a race in it.

**A rejected password does not spend the link.** The twelve-character minimum is
enforced by `hash_chosen_async` — the only function that can hash a chosen
password — and it runs *before* the token is burned. Sending someone back to
their inbox because they typed eleven characters is a reset flow people abandon.

**A successful reset revokes every session, twice over.** `set_password` moves
`changed_at`, which `live_session` already refuses sessions older than, and
`revoke_all_sessions` — dead code until now — marks the rows so a user reading
their session list sees them gone. One closes the door for any path that
forgets; the other makes the closure visible.

**D-046, settled in code.** `casual-task-infra::mail` is the first thing in that
crate: a `Mailer` trait, an SMTP implementation, and a no-op that logs when
`TF_SMTP_HOST` is empty — which [48](48-DEPLOYMENT-PROFILES.md) makes a
supported deployment, not a degraded one. STARTTLS is the **constructor**, not a
setting: `starttls_relay` refuses to send when the relay does not offer it, and
verifies the certificate chain and hostname. There is no key that weakens
either.

**The licence gate shaped the dependency, exactly as it did for D-050.**
`lettre`'s `builder` feature pulls `quoted_printable`, which is `0BSD` and not
on `deny.toml`'s allow-list; its `rustls-tls` feature pulls `webpki-roots`,
which is the `CDLA-Permissive-2.0` crate that turned the database TLS feature
off. Both were answered by feature selection rather than by widening the
allow-list: `tokio1-rustls` + `rustls-native-certs` reaches the platform trust
store, and the twelve header lines this system's only outbound mail needs are
composed in `casual-task-infra`. `cargo deny check licenses` passes unchanged.
The cost is stated where the code is — no MIME encoder, so a subject must be
ASCII and `TF_SMTP_FROM` must be a bare address, and both are **refused** rather
than mangled.

**The reset link never reaches a log and never reaches the table.** `Message`
keeps its body private and prints `<redacted>` from `Debug` — the same mechanism
as [46](46-OBSERVABILITY-AND-OPERATIONS.md)'s `Redacted<T>`, written in
miniature because importing it would add a DAG edge ADR-003 makes an ADR. The
row stores a selector and a salted hash, and an integration test reads the
stored columns back and searches them for the token, which is
[40](40-IDENTITY-AUTH-AND-SESSION.md)'s token-hash gate applied to reset links.

Nine integration tests, against a real PostgreSQL. Every one fails without its
code: the unknown-address response is compared byte for byte against the real
one, a token works once and the second use is 401, an expired token is 401, a
forged verifier against a real selector is 401, a successful reset kills a live
session *and* the cookie stops authenticating, a short password is 400 with the
link still working, asking twice leaves only the newest link live, and a request
for an address with no account still writes its `auth_event` row.

**Not `Gated` at the time this landed, and the reason:** the enumeration
acceptance gate in [40](40-IDENTITY-AUTH-AND-SESSION.md) covers login, reset
**and invite**, and invitations did not exist yet. *That third leg has since
landed — see below.* The timing assertion here is an order-of-magnitude envelope
rather than a statistical one, because a tight bound on a shared CI runner is a
flaky test rather than a stronger one. The reset mail is plain text only —
[29](29-NOTIFICATIONS-AND-DELIVERY.md) §Email content asks for "plain text and
HTML, both readable" of *notification* mail, and an HTML part here would add a
rendering surface without adding information to a message whose entire content
is one URL.

**Invitations are in, and the enumeration gate is closed.**
[40](40-IDENTITY-AUTH-AND-SESSION.md) §Acceptance gates names three endpoints —
"login, reset, and invite responses are indistinguishable for existing and
non-existing accounts, in body, status, and timing envelope". Login closed the
first, password reset the second, and this closes the third. All three legs now
have a test that compares the two responses **byte for byte** plus a timing
envelope, so the gate is a suite rather than a sentence.

`docs/40` §Invitations states the rule this endpoint had to satisfy: "The
response is identical whether or not the address has an account — only the
delivered email differs." `POST .../invitations` therefore returns `202` with a
**constant** body on every path: new address, existing account, already invited.
The cost is stated and it is real — an inviter does not get the invitation id
back and must `GET` the list to find it. It was taken because returning the
created row makes the response a function of the state the caller is otherwise
probing, and every future edit to that body becomes a chance to reintroduce the
oracle. A constant cannot drift into one.

**Tied to the address.** An invitation is not a bearer token for whoever holds
the link: accepting while signed in as an account whose email differs from the
invited address is refused, and a test forwards a link to a signed-in bystander
to prove it. Without that, forwarding the email — which people do, in good
faith — hands membership to the wrong person and the trail records it as the
invitation working correctly.

**Migration 0022 is the seam ADR-032 named this table for.** Accepting is the
strongest form of the pre-workspace problem: the caller may have no account at
all, so there is no `WorkspaceScope` and there cannot be one until the
invitation says which workspace. This is the failure C-002 shipped and fixed in
migration 0020 — read unscoped as `taskforge_app` the policy hides every row and
every acceptance fails, while every test passes because the harness connects as
the owner and RLS is inert for a superuser. Three functions, each `SECURITY
DEFINER` with a pinned `search_path`, a fixed projection and `EXECUTE` to
`taskforge_app` alone, all keyed on the **selector** — never a workspace id or
an email — so none can enumerate a workspace's invitations or answer whether an
address was invited. Single use lives in `consume_invitation`'s `WHERE` clause
rather than in the application, so the predicate cannot be separated from the
write.

**Inviting with a role is gated; inviting is not — and neither half is
invented.** An invitation carrying a `role_id` is a **deferred grant** at
workspace scope, and [04](04-RBAC-AND-AUTHORIZATION.md) says `role.assign`
"grants an existing role at workspace scope". So the endpoint requires
`role.assign` and applies control 1 — "you cannot grant what you do not hold" —
permission by permission against the closed registry, with an unrecognised
permission string failing closed. Without it, inviting would hand out a role the
inviter does not hold, which is the escalation hole D-049 split `role.assign`
from `role.manage` to prevent.

Issuing a **bare** invitation requires workspace membership and nothing more,
exactly as adding a member directly already does, and for the same recorded
reason: **D-054** is `Open`, the closed registry has no invitation permission,
and [04](04-RBAC-AND-AUTHORIZATION.md) names none. Inviting is adding a member
by another route, so it inherits that decision rather than pre-empting it.
D-054's row is extended to say so — when it is Accepted, one mapping change
covers membership, teams **and** invitations rather than three.

Acceptance issues **no session**. It proves control of a mailbox, not of a
password, and signing the caller in would turn a forwarded email into an
authenticated session — the attack the address check exists to stop. An account
created by acceptance gets no credential row; the invitee sets a password
through the reset flow above, which is the same journey someone who forgot
theirs takes.

Fourteen integration tests, every one failing without its code. Still to come
before C-001 is `Gated` at the time this landed: MFA enrolment and the
break-glass path. *Both have since landed — see below.*

**MFA is in: enrolment, step-up, recovery codes, replay refusal and
break-glass.** `casual-task-identity::mfa` had the primitives from the start and
nothing consumed them; `session.mfa_satisfied_at` and `auth_method` had been
columns nothing wrote. This is the layer that uses all of it.

**Replay refusal is a `WHERE` clause, and it is the reason `Totp::verify`
returns a step.** RFC 6238 §5.2: a code is valid for a whole 30-second window,
so an attacker who observes one — over a shoulder, through a phishing proxy —
can present it inside that window. `Totp::verify` was written returning the
matched **time step** rather than a bool precisely so a caller could refuse a
step it had already accepted, and its doc comment said so; nothing could,
because there was nowhere to remember one. Migration 0026 adds
`mfa_factor.last_step`, and `mfa::accept_step` is an `UPDATE ... WHERE last_step
IS NULL OR last_step < $2`. In the predicate, not a read-then-write: two
requests carrying the same observed code would both pass a read-side check
before either wrote, which is exactly the race the attacker is in.

**Monotonic, not a set.** Refusing every *earlier* step as well as the exact one
closes the window on a code captured seconds ago and presented after the clock
ticks on. A per-step set would be bigger, need sweeping, and permit the replay
it exists to stop.

**An unconfirmed factor satisfies nothing.** `confirmed_at IS NOT NULL` lives in
`mfa::confirmed_factor`'s query, not at its call sites, so no handler can forget
it. Migration 0016's own comment named this failure two phases ago: "a user who
lost the enrolment halfway would otherwise be locked out by a factor they do not
have." A test enrols, abandons, and asserts both halves — entry is refused *and*
a code from the unconfirmed factor does not satisfy the step-up.

**Step-up is at workspace resolution, not at login** ([40](40-IDENTITY-AUTH-AND-SESSION.md)
§Workspace-level SSO and MFA step-up). The session is user-scoped —
`user_account` is the only table without a `workspace_id` — while enforcement is
per workspace, so a login has no single policy to apply. The check sits in
`WorkspaceMember`, **after** the membership check: a stranger probing workspace
ids must get the same 404 whether or not the workspace demands MFA, or the
refusal becomes the enumeration oracle the membership check exists to prevent. A
test compares a real requiring workspace against an imaginary one.

**The anti-lockout rule is enforced, not documented.** `docs/40`: "the enforcing
admin must already have MFA enrolled, so nobody can lock themselves out while
locking others in." Enabling the requirement without a confirmed factor is
refused; disabling it carries no such check, because it can only widen access
and demanding a factor there would be the same lockout with the opposite sign.

**Break-glass is a command, not a route** (`--break-glass-clear-mfa <email>`,
[50](50-RUNBOOKS.md) RB-08). An HTTP endpoint that removes a second factor is a
backdoor with a URL: whatever it demanded would be either something the
locked-out owner cannot produce, which defeats the purpose, or something an
attacker could, which defeats the factor. There is no third option, so the
authority is one the network cannot reach — possession of `DATABASE_URL`. It
writes the `auth_event` **before** the delete, so the failure mode is a recorded
attempt rather than an unrecorded removal, and it creates no session, resets no
password and grants nothing. The integration test runs the **real binary**,
because a documented recovery path nobody executes has rotted by the time it is
needed and the argument parsing is the part that rots first.

**The secret never reaches a log.** It is the one recoverable plaintext in the
schema (migration 0016 says so, because TOTP recomputes codes from it). It is
returned once, by `begin`, and wrapped in
[46](46-OBSERVABILITY-AND-OPERATIONS.md)'s `Redacted<T>` everywhere else.
`EnrolmentStarted` and `RecoveryCodesIssued` carry hand-written `Debug` impls
that print `<redacted>` — the workspace lint requires a `Debug`, and a type with
none invites the next person to add the derive.

`TF-AUT-0014` was added to [20](20-ERROR-CODE-REGISTRY.md) through the process
that document defines for adding one. `TF-AUT-0005` and `TF-AUT-0006` were
already registered and unused, and are now what the step-up and the code refusal
return — 401 for both, which is the registry's assignment and the right shape: a
statement about the credential being incomplete, not about missing authority.

Eleven integration tests. **Not `Gated`:** the remaining `docs/40` acceptance
gates for C-001 are the SSO suite against a Keycloak container — SSO is Phase 2 —
and the lockout test, which the login backoff has but no CI job asserts.
**D-064** records the one thing `docs/40` does not decide: how long a step-up
lasts. None is applied, and the instant is stored so a lifetime is one
comparison away.

**F-009 is `Gated`.** It was `Built` because the crate was a registry of names
with no way to record a value — declared metrics that nothing could emit.

`Recorder` records counters, gauges and histograms and renders the Prometheus
exposition body. **It was written here rather than taken from a crate**, and the
reason is the crate's own invariant: `LabelValue` has no `From<String>`, no
`From<Uuid>` and no constructor taking `impl Display`, so a workspace id cannot
become a label. Every general-purpose metrics facade takes labels as `&str`
pairs; putting one underneath this crate would turn a compile error into a
convention at the call site. The cost — owning the exposition format, the
buckets and the concurrency — is stated in the module.

**One third of that cost came due.** The first recorder put a single
process-wide mutex around one map, and every HTTP request took it twice — the
RED counter and the duration histogram — while `GET /metrics` took the same one
and held it across the whole render. The metrics layer was a serialisation point
on the path it exists to measure, and a scrape stalled every request in flight.
It is now sharded, atomic, and snapshot-then-format, with the ordering rules and
the stated cost in [46](46-OBSERVABILITY-AND-OPERATIONS.md) §The recorder is on
the request path. Three tests hold it: N threads × M increments render as exactly
N×M, a scrape running throughout loses no observation, and no scrape taken
mid-observation sees a histogram whose cumulative counts go backwards.

Buckets are chosen against [30](30-PERFORMANCE-AND-CAPACITY-TARGETS.md) rather
than copied: p95 read < 150 ms, so there is a boundary *at* 0.150 and four more
between 50 ms and 350 ms. A layout jumping 0.1 → 1.0 would put the number this
project is judged on inside one bucket, and every quantile across it would be an
interpolation.

The loop now emits `outbox_lag_seconds`, `outbox_dlq_depth` and
`outbox_dispatch_total`, and the C-011 acceptance test asserts they appear —
including that lag returns to **zero** when the queue drains. A gauge that stops
being written keeps reporting its last value, so a backlog that cleared would
show as a backlog forever.

**The guard caught something real while wiring it up.** Consumer names round-trip
through the database as runtime `String`s, and `LabelSet::with` accepts only
`&'static str` — so the compiler refused them. That is not an inconvenience:
[34](34-PLUGIN-AND-EXTENSION-ARCHITECTURE.md) lets a **plugin** subscribe, so
consumer names are open at runtime and passing them straight through would have
grown a series per installed plugin. They are now mapped back to the declared
`CONSUMERS` entries, with anything else collapsing to `other`.

The same problem has no such fix for event types, so `outbox_dlq_depth` is
**not** broken down by one. There is no closed event-type registry to map a
runtime string back to a source literal — the permission set has one, event
types do not — and adding the label without it would put an unbounded value on a
metric. Opened as **D-053** rather than resolved here; RB-02 still groups by
event type in SQL, where cardinality costs nothing.

Serving `/metrics` over HTTP is **not** here: [19](19-WORKSPACE-SCAFFOLD-DESIGN.md)
puts every HTTP type in `casual-task-api` and `casual-task-lint` enforces it, so
the endpoint arrives with C-001. This produces the body it will send.

Two Phase 0 rows remain `Built`: F-007 needs a reference machine, F-011 is prose
with no behaviour behind it. **F-013 needs a human** — the threat-model review
records that an agent conducted it and asks to be countersigned before the
Phase 1 gate.

**F-014 is `Gated`.** It was `Built` with the reason recorded as "prose that no
gate beyond link resolution can hold". That was wrong, and this session proved
it: migration 0013 dropped three columns from `outbox_event`, and **every query
in RB-01 and RB-02 stopped working**. Both runbooks were silently broken and
were repaired only because someone noticed while editing something else.

`scripts/verify-runbooks.sh` now runs all 25 queries the document marks
`✅ executable` against a freshly migrated schema, and fails naming the step.
Two things it does deliberately:

- Every query runs inside a transaction that is **rolled back**, so a runbook
  may contain an `UPDATE` — RB-02's replay does — without the gate mutating
  anything.
- Queries taking bind parameters (`$1` — a workspace id, a correlation id) are
  validated with `PREPARE` rather than executed. That parses and plans the
  statement, resolves every table and column, and infers the parameter types,
  without inventing a value. Executing with a made-up id would have proved less.

Verified by reintroducing the exact historical breakage — a reference to
`outbox_event.dispatched_at` — and confirming the gate names the step, the
column, and the fix. What it does **not** check is whether a query answers a
useful question; it checks that it still matches the schema, which is the
failure drift actually produces.

The remaining four Phase 0 rows are `Built` for reasons that still hold: F-007
needs a reference machine, F-009 needs an exporter, F-011 is prose with no
behaviour behind it, and **F-013 needs a human** — the threat-model review says
an agent conducted it and asks to be countersigned.

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

Still missing before `Gated`: five of the six consumers (C-013, C-016 and the
Phase 3 pair — `sse_fanout` landed with C-015), the sweep's scheduling under a
leader lease, and metric *emission* —
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

**Four cost defects in the dispatch path, found by audit and fixed.** Each was
correct and each got slower with the size of the thing it was measuring — the
shape a load test finds and a unit test never does.

- `outbox_lag_seconds` was an aggregate over the pending set, joined to
  `outbox_event` for `min(created_at)`, recomputed on **every poll** and read
  **inside the claim's transaction**. At a two-million backlog that is two
  million random heap fetches per poll, inside the transaction holding the
  claimed rows' locks — and the loop polls without sleeping precisely when the
  backlog is largest. It is now sampled once per `metrics_interval` (5 s) in its
  own transaction, and reads `outbox_delivery.created_at` instead of the event's,
  which makes the whole gauge an index-only scan of
  `outbox_delivery_pending_ix`. The two timestamps are the same instant — one
  transaction, both defaulting to `now()` — and that invariant is now asserted
  against a real database rather than assumed.
- `outbox_dlq_depth` was counted on every poll too, over a set that
  [25](25-EVENTS-OUTBOX-AND-AUDIT.md) forbids sweeping and that therefore only
  grows. Same cadence fix; `outbox_delivery_dlq_ix` now leads with `consumer` so
  the count is index-only. The residual cost is stated rather than removed: it is
  still O(dead letters), and a maintained counter that can disagree with the
  table is a worse thing to page on.
- The claim query's ordering anti-join had no index that could answer it —
  `outbox_event` was left with only its primary key when 0013 dropped 0007's two
  partial indexes. Migration [0018](../migrations/0018_outbox_dispatch_indexes.sql)
  adds `outbox_event_aggregate_ix`. The plan gate did not catch this because the
  planner's fallback is a hash anti-join, not a sequential scan; the new
  `tests/explain/queries/21` probe binds the subquery to one aggregate, which is
  how the planner actually evaluates it, and that probe **does** fail without the
  index.
- `Dispatcher::assume` re-queried `pg_roles` inside every transaction, including
  one per delivery outcome. The check is a misconfiguration check and a
  misconfiguration is a startup fact, so it now runs once and produces an
  unforgeable `DispatcherRole` token. The check is not weakened: the token cannot
  exist without it, and a compile-time assertion in `outbox.rs` fails if the
  round trip is ever moved back into the constructor.

**C-003 is `Built`.** The resolution core is implemented in
`casual-task-authz` — the scope containment chain, the additive union, the
closed five-constraint set, `allows`, and `explain` — with 17 tests and no
database, which is what [19](19-WORKSPACE-SCAFFOLD-DESIGN.md) isolates that
crate for. A bounded 60-second read cache keys every answer by workspace,
principal id and type, optional project, and `authz_epoch`; mutations bypass it.
The cache exports the D-047 hit ratio and every tenant resolution records its
duration. A full 100-task page is gated to one resolution. It did **not** depend on D-032: the resolver takes an
already-authenticated actor, so the auth mechanism could be settled separately —
and was. D-056's built-in-role matrix keeps the row at `Built` rather than
`Gated`; the resolver, cache, scope ceilings, property tests and explain surface
are implemented and run in blocking CI.

**C-020 is `Building`, and it is the auth class only.** `POST /api/v1/auth/login`
had no limit of any kind. The per-account backoff in `casual-task-identity`
slows an attacker guessing one password repeatedly and does nothing against
credential stuffing — one attempt each against ten thousand accounts never
increments any single account's counter — so
[40](40-IDENTITY-AUTH-AND-SESSION.md)'s "rate limited per account **and** per
IP" was half true.

What landed is the per-IP half of the auth class:
[21](21-API-LIMITS-AND-QUOTAS.md)'s 10/min with a burst of 5, as a token bucket,
over `/api/v1/auth/login`, with `TF-LIM-0001` and a `Retry-After` the constructor
will not let a call site omit. `rate_limit_hits_total` now has something behind
it for the first time, labelled `scope_kind` only.

What did **not** land, stated so it is not mistaken for finished:

- **The other five classes.** Reads, writes, search, bulk and invites are keyed
  per `(workspace, actor)`, which needs an authenticated actor. The limiter runs
  before authentication, so those are a separate piece of work, and
  [21](21-API-LIMITS-AND-QUOTAS.md)'s "Rate-limit isolation" acceptance gate —
  exhausting one workspace's bucket does not affect another's — cannot be
  written until they exist. That gate is why this row is `Building` and not
  `Gated`.
- **Shared state.** The limiter is in-process, because
  [48](48-DEPLOYMENT-PROFILES.md) Profile 1 has no Redis and must work. With more
  than one API instance the limit is per instance and the deployment admits N
  times the configured rate. Profile 2 already says Redis is required at ≥ 2
  instances; nothing enforces it, and nothing warns at runtime.
- **`D-042` stays open.** The metric carries no tenant label at all here, not
  because the contradiction in [46](46-OBSERVABILITY-AND-OPERATIONS.md) §Domain
  metrics was resolved, but because an endpoint whose entire purpose is that the
  caller has no identity yet has no tenant to attribute. Attaching a bucket would
  have meant inventing one.

Two things were found while building it. The first version kept `tokens: f64` and
refilled by `elapsed × rate`; `6 s × (10/60)` is `0.999999999999999`, so the
token [21](21-API-LIMITS-AND-QUOTAS.md) says arrives after six seconds did not,
and the limit was quietly stricter than the document in a way no reading of the
code would show. It is integer duration arithmetic now, and two tests pin the
published numbers. The second is that `client_ip` is duplicated from
`casual-task-api/src/auth.rs`, where it is private; the copy is marked as
temporary in both directions and the follow-up is to delete the one in `auth.rs`.

**C-015 is `Gated`.** Two people on the same board now see each other's
changes. The path is the one [25](25-EVENTS-OUTBOX-AND-AUDIT.md) was built for:
`UnitOfWork::record` writes the event, the dispatcher claims it, and
`sse_fanout` — the first of the six consumers to exist — publishes it to an
in-process hub that `GET /api/v1/stream?project_id=` subscribes to.

Three things are worth reading before the code:

- **The event now carries its project.** Migration
  [0023](../migrations/0023_outbox_event_project.sql) adds `outbox_event.project_id`.
  Without it a fan-out consumer cannot answer "who may see this?" — readability
  is decided at the project, and the alternatives were re-reading the aggregate
  (a round trip per event, and wrong for a delete) or trusting a JSON field no
  schema enforces. NULL means workspace-level and is **not** a wildcard: an
  event that cannot prove which project it belongs to reaches no project stream.
- **A constrained reader is refused, not filtered.** `GET /tasks/{id}` evaluates
  a permission against the task in front of it; a stream has no task in front of
  it, and an outbox event does not carry assignees or an environment. So
  `sse::authorize` asks the permission twice — once for the best-case task and
  once for the worst — and admits the subscriber only if they may read *every*
  task in the project. An actor with an `assignee_is_actor` grant gets `403` and
  polls. That is fail-closed, and it needs no list of constraint kinds to stay
  correct as `casual-task-authz` grows.
- **The per-subscriber queue is 64 with a stated overflow policy** (D-040): the
  slow subscriber is disconnected and *told* it was, so it reconnects with
  `Last-Event-ID` instead of silently carrying a hole. The first version could
  not deliver that notice — it was sent on the channel that had just proved to
  be full — so the client saw a clean end-of-stream and had no reason to resume.
  One reserved slot fixes it; a test asserts the notice arrives.

**Revocation now reaches an open stream.** The shortfall this row declared —
a revoked session's stream stayed open — is closed, in the shape the declaration
predicted: a per-subscription cancel handle plus a tick that re-asks both
questions.

- **Is the credential still live?** Every tick, through
  `middleware::authenticate` — the same function the extractors use, because a
  second implementation of "is this credential still live" is how one door stays
  open after the others close.
- **Is the authority still sufficient?** Only when `workspace.authz_epoch` has
  moved. [04](04-RBAC-AND-AUTHORIZATION.md) defines that counter as bumped by any
  grant, role, team or project membership change *in the same transaction as the
  change*, so an unchanged epoch is proof — not a guess — that re-resolving would
  give the same answer. That makes [05](05-API-SPEC.md)'s "membership is
  revalidated on every `authz_epoch` change" literally true, and it is why the
  expensive check can be rare.

**The window is 15 seconds and the cost is stated where the constant is:** a
session destroyed just after a tick keeps receiving events for up to that long,
and every open stream costs one session read plus one epoch read per tick —
roughly 133 queries a second per 1,000 streams, and ~1,330 at the hub's
10,000-subscriber cap. That is the dominant database cost of live updates, it
scales with connections rather than events, and the way out is a shared
invalidation signal (`LISTEN`/`NOTIFY`) rather than a smaller number.

A tick that cannot reach the database does **not** cancel: a blip would otherwise
drop every stream in the deployment at once, which is a self-inflicted outage
arriving exactly when the system is already unwell. It fails closed on every
answer that is an answer.

`sse_connections_active` no longer counts a stream that has been cancelled but
not yet dropped — the release is idempotent and runs at cancellation, so the
gauge does not read high during precisely the incident an operator is watching it
for.

**`Last-Event-ID` replay and the 100 ms window are now built**, which was the
last of what [05](05-API-SPEC.md) §Live updates specifies.

Replay matters more than it looks. Every "reconnect with `Last-Event-ID`"
message this feature emits — the lag policy, the revocation notice, a dropped
socket — was advice that did not work, because nothing stood behind the header.
A recovery instruction that does not recover is worse than none, because the
client stops looking. It is bounded to 5 minutes / 1,000 events per
[05](05-API-SPEC.md), **and to 1,024 topics**, which that document does not
mention: a per-project history is a map a user can grow by opening projects, and
1,000 events each is only bounded memory if the number of histories is bounded
too. All three overflow into the same answer — "you lost events, refetch" —
because a client past the window must never be handed the tail of a history and
left to assume it was the whole of it.

Coalescing collapses per **(aggregate, event type)** rather than per aggregate.
Narrower than [05](05-API-SPEC.md)'s wording, deliberately: collapsing
`task.created` into the `task.updated` 20 ms behind it hands a subscriber an
update for a task it has never heard of, which is
[25](25-EVENTS-OUTBOX-AND-AUDIT.md)'s ordering guarantee undone at the last hop.
A drag emits one event type repeatedly, so the case that document is about is
fully covered. Flagged rather than settled silently — if the intent was to
collapse across types, one line changes it.

**C-015 is `Gated`.** The gap this row declared — nothing drove
`GET /api/v1/stream` over HTTP and read frames off the body — is closed by
`crates/casual-task-api/tests/sse_stream.rs`, which CI runs with every other
Docker-backed suite. It asserts the *assembly* the per-mechanism tests could not
see:

- a subscriber receives a live frame, carrying the event's own id as `id:` —
  which is what a client sends back as `Last-Event-ID`;
- a forty-event drag on one task arrives as **one** frame with the final
  payload, and no second frame follows;
- a reconnect with `Last-Event-ID` replays exactly what was missed, in order,
  **before** any live frame, and the stream then keeps delivering;
- a stream is fed its own project and nothing else — asserted against both a
  same-project-different-workspace topic and a same-workspace-different-project
  one, which is the pair a comparison that dropped either half would leak
  through.

Each of those is a bounded wait on bytes rather than a status code, because the
interesting half of "one frame" is the frame that must **not** arrive. Two of the
four carry a counterweight publish afterwards, so an assertion about silence
cannot pass against a stream that is simply dead.

The fixture shares **one** `AppState` between the router and the publisher. Every
other test file in this crate builds a router per caller, which here would mean
publishing into a hub nobody is subscribed to and asserting that no frames
arrive — a false pass, and the reason that choice is commented at the fixture
rather than left to be re-derived.

**C-001 is unblocked.** ADR-032 is Accepted, which is what this document's
`Accepted` requires — "design final **and** its ADRs Accepted". It carries two
schema changes into C-001's first migration: `api_token.token_hash` becomes
`token_selector` + `verifier_hash`, and the auth-storage tables are added with a
written exemption in migration 0010's block. `principal_type` is deliberately
**unchanged**.

**C-006 is `Gated`, and the read/create half of C-008 is in.** Before this, the
product could log in and log out. It can now create a project, create a task in
it, and read either back:

```
GET   /api/v1/projects              list       (cursor)
POST  /api/v1/projects              create     (Idempotency-Key required)
GET   /api/v1/projects/{id}         read       (returns an ETag)
PATCH /api/v1/projects/{id}         update     (If-Match required)
POST  /api/v1/projects/{id}/tasks   create     (Idempotency-Key required)
GET   /api/v1/tasks                 list       (cursor)
GET   /api/v1/tasks/{id}            read       (returns an ETag)
```

The gate is `cargo test --workspace -- --ignored` in CI: fourteen tests against
a real PostgreSQL 16, each of which fails without the code it covers. Plus the
two new indexes in the `schema` job's required list, and three new `EXPLAIN`
probes in `explain-no-seq-scan` (23 queries, no sequential scan).

**Authority comes from `role_assignment` and nothing else.** Migration 0003 says
so — "not by a boolean column, not by an `is_admin` flag, and not by project
membership" — and the handlers ask the C-003 resolver rather than checking
membership. The consequence is stated plainly rather than worked around: **a
workspace with no `role_assignment` row can create nothing.** Visibility still
confers read ([04](04-RBAC-AND-AUTHORIZATION.md): "visibility alone confers
`project.read` and `task.read`"), so a member with no grants sees the workspace
and cannot change it. That is the design behaving correctly and it is also a
product that cannot be used end to end until something seeds the first grant.
Recorded as **D-054** rather than resolved here: nothing in the design record
says how a workspace acquires its first owner, and inventing an answer in a
project handler is exactly what AGENTS.md forbids. It belongs with C-002.

**One judgement call, named.** A project create writes a `project_membership`
row for its creator. `project_membership` conveys belonging and never capability
(migration 0003), so this grants nothing — but without it, creating a `PRIVATE`
project produces something its author cannot read back, and "create then 404"
is a bug rather than a policy.

**Three things the design record was silent or wrong about, found by building
this:**

- **The cursor had no cast.** `casual-task-persistence::compile` emitted
  `(t.updated_at, t.id) < ($3, $4)` with both bound as text. `timestamptz < text`
  is not an operator PostgreSQL has, so **every second page of every list would
  have failed at execution time** — and no test could see it, because the whole
  C-012 suite asserts the compiler's *output*. The same class of bug as the one
  the read-path exercise found above, in the one place that exercise did not
  reach. Fixed by casting the parameter (never the column, which would defeat
  `task_list_ix`), with an exhaustive `cursor_type` so a new sortable field
  cannot be added without deciding how its cursor resumes.
- **`SELECT t.*` cannot be decoded.** The compiler's projection returned `type`,
  `priority` and `state` as PostgreSQL enums, which no `String` decoder accepts.
  The compiler now selects the repository's explicit column list, cast to
  `text` — one projection, defined once, used by the compiler and the single
  read alike.
- **`docs/24`'s 409 body cannot be produced.** §The conflict response specifies
  `conflicting_fields` and `your_safe_fields`, which need the pre-image the
  caller was editing. No request carries one and no table stores one. The
  response carries `your_version`, `current_version`, `changed_by`, `changed_at`
  and the full current representation; the two field lists are **absent**, not
  approximated, because a wrong "these fields are safe to retry" is acted on
  automatically by the client. **D-055**.

**Also in, because C-006 could not exist without them:** the default workflow
from [23](23-WORKFLOW-AND-STATE-MACHINE.md) §The default workflow, provisioned
by the first project create in a workspace and guarded by a partial unique index
(migration 0019) — the C-007 half that is data rather than behaviour; the
idempotency protocol from [24](24-CONCURRENCY-AND-IDEMPOTENCY.md) §Idempotency
for creates, claim and all, against the `idempotency_key` table that has existed
since migration 0008; and lexicographic board ranks (ADR-013), append-only, with
the minimum character reserved so a midpoint always exists for the drag path
that arrives with C-018.

**The rest of C-008 is in: update, delete, transitions, assignees and tags.**
A task could be created and then never changed; now it is a work item.

`PATCH` and `DELETE` require `If-Match` (428 absent, 409 stale, and the `409`
body carries the current representation so the loser can merge). `PATCH`
**declares** `status_id` and `state` in order to refuse them with
`TF-WFL-0001`: leaving them out would make `deny_unknown_fields` call them
fields nobody has heard of, when the truth is that they exist and have their own
door ([23](23-WORKFLOW-AND-STATE-MACHINE.md) §The transition command).

**The transition handler does not re-derive the state machine.** Steps 1–3 of
[23](23-WORKFLOW-AND-STATE-MACHINE.md) §Validation order are the handler's
(readable → 404, version → 409, `task.transition` → 403); steps 4–7 are
`casual-task-workflow`'s `validate`, which was already implemented and tested
and already returns the **first** failure in the fixed order. The handler
supplies the facts — the actor's held permissions on the project, the blockers
they can see — and maps each `Rejection` onto its documented code. The order is
covered by a test that violates several rules at once per request and asserts
which one is reported; a handler checking permission before version, or version
before visibility, passes every single-violation test and fails that one.

Two behaviours from [23](23-WORKFLOW-AND-STATE-MACHINE.md) that are easy to miss
and are both tested: a move to the status the task already occupies is a `200`
no-op that writes **no** event, which is what makes retries safe without an
idempotency key; and leaving a terminal state writes `task.reopened` rather than
`task.status.changed`, because "how often does work come back?" cannot be
answered from a generic status-change event.

**Step 8 — plugin `validation.transition` hooks — is not implemented**, and
nothing fakes it. It needs the plugin runtime (Phase 3,
[34](34-PLUGIN-AND-EXTENSION-ARCHITECTURE.md)).

**`fields` validates and is then discarded.** It satisfies step 6, and storing
the values needs custom-field value storage — **D-033**, deliberately deferred
to before Phase 3. The default workflow requires no fields, so no path in the
product reaches the gap today; a custom workflow that required one would
validate correctly and record nothing.

**`TF-TSK-0005` is read as visibility, not as a membership row.** "Assignee is
not a member of the project" cannot mean `project_membership` for a
`WORKSPACE`-visible project — the default — because such projects usually have
no membership rows at all, so the rule would be unsatisfiable in the common
case. What is enforced is [04](04-RBAC-AND-AUTHORIZATION.md) §Visibility's own
predicate: **work is never assigned to someone who cannot see it**. A stranger,
another tenant's member, and a colleague who cannot open the project are refused
identically.

**Tags are applied by id, not created by name.** Authoring the tag vocabulary is
`tag.manage` and belongs to a tags endpoint that does not exist yet; accepting a
name here would make every typo a new tag. Applying an existing tag is a change
to the *task*, so the permission is `task.update` rather than `tag.manage`.

Dependencies and the complete C-012 filter path have since landed. Dependency
insertion closes the cycle check inside the guarded statement under a
workspace-scoped advisory lock; task lists attach assignees for the page in one
query. C-008 is `Built`, and the generated transition invariant now proves over
1,000 deterministic cases that the stored status and permanent state move as a
pair. It is not `Gated` until the wider task acceptance contract has one named
blocking suite.

**The lifecycle is on screen** — the custody panel and the environment board.

**The custody panel is the hand-off, not a history.** The moment anyone opens a
task is usually the moment they are about to pass it on, so the actions are the
panel and the log sits under them as evidence. The transfer control says *"this
clears the current assignees, so it lands in that team's queue"* above the
button rather than in a toast after it — the clearing is the point of a hand-off,
and a person who did not expect it has lost their assignee. A failure cannot be
recorded without evidence, so that button is disabled rather than the refusal
being explained afterwards.

**The trail is merged, not stacked.** Three lists would make the reader
interleave them by timestamp, which is the work the panel exists to do. It reads
as sentences — *"handed it from Android to Backend — API returns 500 on rotate"*,
*"failed it on qa — still crashes on rotate"* — and the two numbers that expose a
broken process, the failure count and the bounce count, are stated rather than
left to be counted.

**The environment board is the surface the product could not produce.** Columns
are environments in deployment order, plus *Not yet deployed*, because a
project's work is not all in the pipeline and the tasks that have reached nothing
are exactly what a release conversation asks after next. It is a read: a card
does not move by being dragged, because an environment changes when something was
*deployed* and a promotion is a record of a real event rather than a wish.

**`team` joins the filter grammar's closed field set.** `is_empty` is the
important operator and not an afterthought: a task owned by no team is untriaged,
and "untriaged in this project" is the queue a lead opens first.

**Two defects found by looking at the screen, neither by a test.**

- *An empty filter value was dropped before it was sent.* `docs/27` says
  "`field=` — the empty value is how a URL says 'unset'", and the client's query
  builder skipped empty values — right for every other caller, and it silently
  turned `environment=`, `team=` and `assignee=` into no filter at all. The
  environment board's "Not yet deployed" column showed every task in the project,
  including the ones it had just listed under qa. An absent key and an empty one
  are different questions.
- *`scripts/dev-up.sh` never applied a new migration.* It skipped the whole step
  whenever `task` existed, so a surviving dev database silently stayed behind the
  code — the symptom was a 500 naming a column that exists in `migrations/`. It
  now records what it has applied and runs only what is pending. Every migration
  since 0030 would have hit this.

**Custody is served** — transfer, promote, verify, and the chain they leave
([45](45-DEVELOPMENT-LIFECYCLE-AND-CUSTODY.md)). Migration 0031 gave the model
the vocabulary; these are the four endpoints that write and read it.

**Why they are commands and not fields.** A transfer *clears the assignees* as
part of the move, because the task has to land in the receiving team's queue —
a `PATCH {"team_id": …}` that silently emptied another field would be the worst
kind of surprise. A promotion writes a log row *with* the column, because "when
did this reach staging" is the question the column exists to serve and a plain
field write answers it with nothing. A verification is not a field at all.

**Not idempotent, on purpose, in two places.** Android → Backend → Android is two
real events, so the transfer log keeps both and the bounce count survives; handing
a task to the team that already owns it is a `409` so a retry cannot inflate it.
A second promotion to the same environment is a redeploy, and a log that
swallowed it would understate the work.

**The invariant is "no silent move", not "one writer".** `PUT
/tasks/{id}/environment` predates all of this, carries `If-Match`, and can refuse
a stale write — a guarantee the promotion path does not offer. So it keeps its
own `UPDATE` and calls `custody::record_promotion` for the log, and a test
asserts the older door leaves a trail. Without that, the history would be complete
or not depending on which endpoint a task went through.

**A verdict is not a status change.** What happens next — back to the developer
on a fail, forward on a pass — is a transition the caller makes afterwards.
Keeping them separate is what lets "failed twice on qa, then passed" survive
however many times the status has changed since; a status column only ever holds
the latest value.

**Ten integration tests**, each on a property a plausible implementation gets
wrong rather than on a row being written: the assignees are cleared, a round trip
is two events, a team not on the project is refused, the column and the log move
together, the older endpoint logs too, and failures accumulate.

**The product had no user research, and the surfaces show it**
([44](44-PRODUCT-RESEARCH-AND-SURFACE-BRIEFS.md)). Three findings, each
checkable: [01](01-ORD.md) §Users lists buyer segments rather than people;
[42](42-FRONTEND-ARCHITECTURE.md) is fourteen technical sections with nothing
about what anyone is doing on a screen; and no persona, job or task flow appears
anywhere in forty documents. [12](12-COMPETITIVE-ANALYSIS.md) is a strong
*strategy* teardown and its own closing section — "Open questions to resolve with
research" — was never resolved.

**The consequence is structural.** With no layer between "build a work tracker"
and "implement the API", the screen inventory became the *endpoint* inventory: a
page for tasks, a page for the board, a settings page each for roles, teams,
workflow and tags, because there is an endpoint for each. A product organised by
its data model can be feature-complete and still read as a rendering of a
database, because no screen can say why it exists. That is the gap between this
and the products it is measured against, and restyling does not close it.

**What 44 adds:** five roles, ten *moments* when a work tracker is actually
opened with the time budget of each, nine jobs ranked by frequency × pain, and a
brief per surface — its one question, what must be legible without scrolling, its
first action, and how it fails. Every claim is marked `[Given]`, `[Pattern]` or
`[Assumption]`, and each assumption is listed again with what would settle it.
It is desk research and a teardown, **not** interviews, and says so: there are no
users yet, and invented quotes would be worse than none.

**Two things it settles that were previously taste.** Seven of the ten moments
have budgets measured in *seconds* — so a glance surface that needs scrolling has
failed the moment, which turns "no scrolling" from a preference into a derived
requirement. And the job ranking shows the screens are built almost exactly
backwards: the most complete surfaces serve the two rarest jobs (administration),
while the highest-frequency job — "what is mine, and what changed" — has a
filtered list that cannot say what changed.

**Two current screens fail the test outright.** "All tasks" with no project scope
serves no moment — nobody's question is "show me every task in the workspace" —
and should become Search results. Tags settings is a destination for a vocabulary
nobody visits on purpose, and belongs inside the picker where the need arises.

**The attachment origin exists, which is what made the pipeline reachable**
(C-010, `docs/28` §Serving downloads). Presign, commit, scan status and download
had all been built; `presign_put` returned `{origin}/attachments/{key}` and
**nothing served that origin**, so a client could obtain an upload URL and had
nowhere to send the bytes.

**A second listener, because a second port is a second origin.** `docs/28` calls
origin separation "the single most important control here: a stored HTML or SVG
file cannot execute in the application's origin even if every other check
fails", and `Config::from_source` already refused a deployment whose
`TF_ATTACHMENT_ORIGIN` shared an origin with `TF_PUBLIC_URL`. That promise was
unkeepable while nothing served the other origin. `TF_OBJECT_BIND_ADDR` binds
it; unset, this process serves nothing there and says so at startup, which is
the S3 profile where the bucket answers instead.

**The signature is the whole authority, and it is bound to the method.** There
is no session at the attachment origin and there must not be — a cookie would
not travel there anyway. A presigned URL is a capability minted by an endpoint
that already checked `task.attachment.create` or `task.attachment.read`. The
signature covers the key, the expiry *and* the method, so a download link handed
to a colleague cannot be turned into an upload slot. Eight tests assert the
properties a bug would leave working: method binding, expiry, three shapes of
forgery, path traversal under a *valid* signature, and the download headers.

**Uploads replace rather than append.** A client retrying a `PUT` whose response
it never saw is doing the right thing; appending would double the bytes and the
only thing that would notice is `commit`'s size check — a refusal for a correct
client. `FilesystemStore::replace` is deliberately not on the `ObjectStore`
trait: with S3 the bucket takes the `PUT` and this process never sees a byte.

**D-062 is settled: fail closed, countersigned 2026-08-10.** With no scanner
configured a committed attachment stays `PENDING` and is never downloadable.
That is not a gap in this work — it is the decision working. A deployment that
wants attachments served must configure a scanner, and the alternative default
was, in `docs/14`'s own words, a silent lie.

**The task surface is complete** — subtasks, blockers and activity render, and
the assignee set is read rather than guessed at (C-008, C-011, C-018). The
endpoints had been served for some time; nothing in the client reached them, and
the surface said so in one grey line.

**Two more write-only holes, the same shape as the last two.**

- `GET /api/v1/tasks/{id}/assignees` — the set was returned by `POST` and by
  nothing else, so the only way to learn who was on a task was to assign someone.
  `TaskView` now carries assignee ids, attached for the whole page in one query
  rather than fetched once per card. The detail endpoint remains the mutable-set
  surface. Ids, not names — the client resolves them through the member directory
  it already holds, and a second name source would be a second thing to keep in
  step with anonymization (ADR-026).
- `DELETE /api/v1/tasks/{id}/dependencies/{other_id}` — dependencies were
  add-only. An edge added by mistake gated the blocked task's transitions forever,
  and the only escape was `task.dependency.override`, which is an authority for
  *ignoring* a real blocker rather than a way to correct a wrong one. A graph you
  can only add to stops describing the work.

**Two decisions the removal needed, made here rather than left implicit.** No
direction parameter: at most one edge can join a pair, because `A blocks B` and
`B blocks A` together are a cycle, so naming both ends identifies it. And the far
end need **not** be visible — `docs/03` shows an unreadable blocker as
`restricted` rather than hiding the edge, so requiring visibility would make
exactly those edges permanent while protecting nothing the caller cannot already
see on their own panel. The authority is `task.update` on the task in the path.

**The attachment-origin gap is closed.** `TF_OBJECT_BIND_ADDR` serves the
filesystem origin with a method-bound signature and the browser preflight now
reaches it. The pipeline — presign, upload, commit, scan and download — is
reachable end to end. S3 remains an explicitly unsupported backend rather than
a configuration value that silently selects filesystem storage.

**A wrong client type crashed the surface, and the test had encoded the same
mistake.** `Relationship` carries `Subtasks` under `#[serde(flatten)]`, so the
payload is `{parent, data, done, total, truncated}` — there is no `children`
object. It was typed as nested, the panel test's stub repeated the assumption,
the test passed, and the first real response threw `Cannot read properties of
undefined`. Found by opening the page. The type is now copied from the payload
and the stub carries a comment saying why.

**Two defects the panels exposed in what was already there.** Both were visible
on every task and neither had a test:

- *Every value in the metadata column rendered as three characters.* `.pop` is
  `inline-flex` and its trigger carries `overflow: hidden` for the ellipsis;
  together those make the trigger a flex item whose automatic minimum size is
  zero, so it collapsed to ~40 px inside a 203 px cell — "Nob…", "N…", "T…". The
  column exists to be read at rest, so this was the surface's worst bug and it
  looked like a design choice.
- *"Attachments is not available yet."* The verb agreed with the *number of
  sections*, and every section name is a plural word. The sentence now names them
  after a colon and has no verb to get wrong.

**What the panels refuse to guess.** Subtask progress is the server's `done` and
`total`, counted across every child the caller may see — a panel counting its own
rendered page would report "3 of 5 done" on a task with forty children. A blocker
in a project the reader cannot see is drawn as restricted rather than dropped,
because a task shown as blocked by nothing reads as "you may move this". And an
activity row whose event type the client has never seen is rendered as its event
type: a dropped row is a hole in an audit trail.

**Activity is fetched only when opened.** On a board where the peek opens on
hover, an eager fetch is one history request per card looked at, and nothing on
screen would show it happening.

**Administration is reachable** (`/settings`, C-018). Seven sections — profile,
workspace, members, teams, roles, workflow, tags — each a route, wired to the
endpoints that already existed and unbuilt in the client until now. Functional
only: the visual design is Codex's, and `webapp/src/styles/settings.css` carries
structure and no decoration for that reason.

**Two reads had to exist first, and their absence was the finding.** The server
could *write* authority and team membership and could not read either back:

- `GET /api/v1/role-assignments` — the grant set was write-only. `POST` created a
  grant and `DELETE` needed its id, and that id appeared exactly once, in the
  response to the call that made it. An admin who closed the tab could never take
  a permission back, and no screen could answer "who can do this here?". Keyset
  on `id` (UUIDv7, so id order is time order), filterable by principal, role or
  scope, and `role.assign`-or-`role.manage` to read — the same pair that may
  assign, because choosing a role safely means seeing what someone already holds.
- `GET /api/v1/teams/{team_id}/members` — a team is a *principal* a grant names
  (`docs/04`), so "who does this grant reach?" was unanswerable through the API.
  It joins `workspace_membership` rather than reading `team_membership` alone:
  that table carries no `workspace_id` and therefore no policy of its own
  (migration 0010), so the join is the tenant boundary rather than a nicety. It
  returns no `joined_at` — `team_membership` is `(team_id, user_id)` and nothing
  else, and the workspace's join date would have been a plausible-looking answer
  to a different question.

**The permission list is provably a copy.** The role editor needs all 29
permission keys and no endpoint lists them, so they are typed out in the client —
and a typed-out copy drifts. `webapp/src/api/roles.test.ts` parses
`crates/casual-task-model/src/permission.rs` and asserts set equality. Both
failure modes are silent in a browser: a key the server has and the client omits
is an authority nobody can ever grant, and a key the client invents is offered,
saved, and refused with `TF-VAL-0005` for a control the product itself drew.

**Two client bugs were found by clicking, not by testing.** Both are now covered:

- *An empty body is not only a 204.* `POST /teams/{id}/members` answers `201`
  with no body, and adding someone already in the team answers `200` with no
  body. `request()` special-cased `204` and handed everything else to
  `response.json()`, which threw a `SyntaxError` — not an `ApiError` — so the
  user was told "something went wrong on the server" about a request that had
  just succeeded. Every write in the product goes through that function.
- *A view must claim its own scroll region.* `.shell__main` is `overflow: hidden`
  and the shell is a fixed-height grid, which is how the rail and header stay
  still. The settings panel did not scroll itself, so the transition matrix
  rendered below the fold and nothing reached it.

**`requestWithVersion` now reads the `ETag`, which its own documentation already
claimed.** It only ever read `version` from the body. `WorkspaceBody` carries no
`version`, so the workspace rename could never have sent the `If-Match` it
requires. A weak validator (`W/"7"`) is refused rather than used: weak means
"equivalent, not identical", which is exactly the guarantee a precondition must
not be given.

**What the screens say rather than hide.** A section the caller cannot administer
renders the sentence naming the permission instead of vanishing from the
navigation — someone told "your admin can change that under Roles" needs Roles to
exist for them to find. The five `docs/04` ceilings are not re-implemented in
TypeScript: the control is offered and the refusal is rendered with the code that
names the rule, because a greyed-out button hides *which* rule was hit.

**Not built here, and said on the screens that would have offered them:** grants
at team, project and environment scope (the API takes them; there is no picker
for the scope id), renaming or deleting a tag (no endpoint), and editing the
wildcard "from anywhere" transitions (they are listed below the matrix rather
than drawn in it).

**Bulk transitions are in** (`POST /api/v1/tasks/bulk`, [05](05-API-SPEC.md)
§Bulk operations). Selection on a board is unbuildable without it: forty cards
dragged to Done is forty conditional requests, and the client has no way to
report what happened to them as one outcome.

**Partial success is the contract, so `207` is unconditional.** `docs/05`:
"Bulk operations across 100 tasks with individual permission and workflow rules
will legitimately partially fail, and all-or-nothing would make the feature
useless." The response is `207 Multi-Status` whatever the mix — all-success and
all-failure included — because a client that must parse per-task results anyway
should not first have to branch on the status line to find out whether it has
any.

**Each task is its own transaction, and the test asserts it from the
database.** A handler that answered `207` with correct counts and rolled the lot
back would pass every status-code assertion, so the partial-success test reads
`task.status` and the three history tables afterwards: the tasks that succeeded
moved and wrote `activity` + `audit` + `outbox`; the one that refused wrote
none.

**The rules are `apply_transition`, not a copy of them.** The single-task
handler was split into an HTTP wrapper and one function holding `docs/23`'s
validation order; bulk owns the transaction and calls that function. A second
implementation of the order would be a second thing to keep correct, and the
order is a *specification* — the first failure is the one a user sees.

**Every success carries the call that reverses it.** A `207` across forty tasks
where six refused cannot be undone by one inverse call, so each result carries
`undo` — the status the task came *from*, and the version it now holds. A test
replays those without remembering anything about the before-state, which is
exactly what a client has. `Transitioned::from_status_id` exists for this: the
moved row no longer knows where it was.

**The envelope is refused whole; anything task-shaped is a row.** An unknown
operation, no tasks, a repeated task, a version for a task that was never named,
or more than the 100 of [21](21-API-LIMITS-AND-QUOTAS.md) (`TF-LIM-0003`) are
`400`, because there is nothing to report per task and the client could have
known before sending. Not found, no permission, stale, blocked, no such
transition — anything that can be true of one task and false of the next — is a
row in the `207`. A missing `if_match` entry is a per-task `428` rather than a
`400`: it is one task's problem, and refusing the batch would punish the other
thirty-nine.

**Above the limit the client is told to split.** `docs/05` directs it to the
async job endpoint, which is C-024 and does not exist; the refusal names the
limit rather than pointing at a URL that would 404.

**The derived-state property is now built.** A deterministic 1,000-case
generator chooses transitions across workflows and asserts every accepted move
carries the destination status and its permanent state together. Database
integration tests still assert the stored row, so a correct response over an
incorrect write cannot satisfy the suite.

**One pre-existing divergence, reported rather than fixed here.** `ApiError`'s
original codes — `TF-REQ-0001`, `TF-REQ-0004`, `TF-SRV-0001`, `TF-SRV-0003` —
use areas that [20](20-ERROR-CODE-REGISTRY.md) does not define, and
`ApiError::forbidden` used `TF-AUT-0002`, which the registry assigns to "session
expired". Every code added by C-006 and C-008 is copied from the registry, so
the two sets disagreed in one direction only. **Corrected under D-055** below,
together with the gate whose absence allowed it.

**D-054 is Accepted, and the product is usable end to end.** It was not a design
question at heart; it was a hole that only looked like one. `role_assignment` is
the only source of authority in the system (migration 0003: "No permission is
granted anywhere else — not by a boolean column, not by an `is_admin` flag, and
not by project membership"), and **nothing created one**. A person could sign up,
create a workspace, create a project in it, and be refused every write to their
own tenant with `403 TF-AZN-0001`, permanently — because the only way to get a
grant is to hold one.

The resolution is `docs/04` §Built-in role templates, executed rather than
invented:

- The five templates are materialized into each workspace when it is created.
  They are **per workspace, not seeded by a migration**, and that is forced by
  the schema rather than chosen: `role.workspace_id` is `NOT NULL REFERENCES
  workspace(id)` and the table carries a row-level-security policy keyed on it,
  so a global template row has no workspace to belong to and would be invisible
  to everyone under the policy.
- The creator is granted **Owner at `WORKSPACE` scope**, in the same transaction
  as the workspace row, its membership row, and the `UnitOfWork::record` history
  (ADR-006). The grant is in the audit record's `after`, which is `docs/04`
  control 7.

**Two of the five sets are literal; three are judgement calls, and they are
listed rather than buried.** `docs/04` gives each template a one-line *shape*,
not a set of keys. Owner is "Everything" — asserted against
`permission::ALL` itself, so a permission added to the closed registry is
carried by Owner without anyone remembering. Administrator is that minus
`workspace.delete` and `workspace.owner`, asserted as a *difference* for the
same reason. Project Manager, Member and Guest are prose, and the cells prose
does not decide are **withheld** — AGENTS.md priority 1, and widening a template
later is additive where narrowing one takes away authority somebody is using.
They are data in `template::UNDECIDED`, with a test asserting each names a real
template and a real key, and another asserting every one of them actually fails
closed. **D-056**, settled by C-004's golden matrix.

The sharpest is Member and `task.close`: `docs/04` says "transition tasks", and
`docs/23` makes closing require `task.close` **in addition to** a valid edge — so
a Member who may transition still cannot finish work. Nothing blocks on it
today, because no endpoint assigns a role yet.

**The state is made unreachable in two directions, by two different
mechanisms.**

- *Creation* — in the type system. `workspace::insert` no longer returns a
  workspace; it returns an `Unowned` whose inner record is `pub(crate)`, and the
  only thing that opens it is `role::bootstrap`. A handler that creates a
  workspace and skips the grant has nothing to build a response from and **does
  not compile**.
- *Removal* — in the database. Migration 0021 implements `docs/04` control 4
  ("the final grant carrying `workspace.owner` cannot be removed or downgraded.
  Enforced as a database constraint check inside the transaction"), as a
  `BEFORE DELETE OR UPDATE` trigger on `role_assignment`. It covers the `CASCADE`
  from `role` too, and it refuses a *downgrade* — moving the last owner onto a
  role without `workspace.owner` — while still permitting a *transfer*, which is
  neither removal nor downgrade and which a naive rule would have made
  impossible.

**The symmetric database constraint was considered and not taken, deliberately.**
"No `workspace` row exists without an owner grant" would be a `DEFERRABLE
INITIALLY DEFERRED` constraint trigger firing at `COMMIT`. Nine call sites insert
workspaces directly — the `EXPLAIN` corpus's 100 workspaces, the 2M-row reference
corpus, the schema gate's own fixtures, four persistence tests — and every one
would have to mint an owner grant. That changes the corpus the
`explain-no-seq-scan` gate plans against, and that gate's value comes from the
corpus being stable. Recorded in migration 0021 so the stronger option stays
visible rather than being forgotten.

**Ten tests, and each fails without the code it covers.** Seven in
`casual-task-persistence` and three through the HTTP route. The persistence ones
hand the written rows to `casual_task_authz::allows` rather than counting them:
rows in a table are not authority, and the claim being made is that the resolver
accepts them. They assert the owner may exercise every permission in the
registry at workspace *and* project scope (the scope chain is what carries the
grant down), that a stranger gets nothing, and that a grant in one workspace does
not reach another.

**C-013 is `Built`.** A user can now find a task by typing words from its
title. Three parts: the projection that makes it possible, the query that reads
it, and the grammar that reaches the rest of the closed field set.

**The projection is an outbox consumer, and it recomputes rather than patches.**
[26](26-SEARCH-INDEXING-AND-QUERY.md) puts the refresh after commit so a task
write never waits on GIN maintenance, whose pending-list flushes are bursty —
the latency spike a drag-and-drop board must not have. The cost is the one that
document already states: search is eventually consistent, structured filters are
not. Recomputation is what makes it safe behind at-least-once delivery: deliver
twice, late, or out of order and the row still converges, which a delta would
not. Asserted by delivering the same event three times and counting rows.

It carries **its own pool as `taskforge_app`**. The dispatch loop runs as
`taskforge_dispatcher`, which bypasses row-level security and is granted on the
two outbox tables and nothing else (migration 0014) — granting it the task
tables would hand a `BYPASSRLS` role every tenant's task text. `Claimed` gained
`workspace_id` so a consumer can rebuild its scope through
`WorkspaceScope::for_job`; every consumer will need it and none can derive it.

**The URL grammar lives in `casual-task-search`, beside the AST.**
[27](27-FILTER-AND-SAVED-VIEW-DSL.md) §Compilation draws one pipeline with two
entry points meeting at the same AST, and a handler that re-derived what `<`
means would be the second one. `<` on a date is `before`, on `priority` it is
`lt`, and the field's own type decides. `status`, `assignee`, `priority`,
`state`, `type`, dates and tags all reach the compiler now; `project_id` is kept
as an alias for the grammar's `project` because it shipped in C-006.

**C-017 is `Built`.** The extension point registry ships in Phase 1 for the
reason [34](34-PLUGIN-AND-EXTENSION-ARCHITECTURE.md) opens with: a seam only
plugins use is a seam nobody has tested, "and we find that out in Phase 1, not
Phase 3". So the core's own drawer panels, card badges, project tabs, palette
commands and settings sections are registered as ordinary `Provider::Core`
contributions and travel the same path a vendor's will. A test asserts every
frontend point has at least one core contribution, so a point that nothing
exercises fails the build rather than waiting for the first third party to find
it.

**The closed set is checked against the design record in both directions.**
Adding a row to either of [34](34-PLUGIN-AND-EXTENSION-ARCHITECTURE.md)'s two
tables is an ADR trigger (ADR-009), and a table that drifts from the code is
worse than no table — it is a contract two teams read differently. The tables
are parsed at test time: a variant with no documented row fails, and a
documented row with no variant fails. The surface (backend or frontend) is read
from *which* table a point is in rather than restated in code, so a point that
moves between them cannot keep the old surface.

**What it deliberately does not ship is the payload.** A panel's load URL, an
action's handler — none of it. Phase 1 has no third party to learn the shape
from, and a compatibility contract guessed a phase early is worse than one
written a phase late. What is fixed now is the part later phases cannot change:
who contributed, to which point, under what name, and what the host does when
the contribution fails.

**ADR-017's fail-open default is a type, not a convention.** `Bounds::for_point`
is the only constructor, so no call site can pick its own timeout and drift from
the 500 ms the document fixes; opting a plugin into fail-closed is spelled
`failing_closed_at_the_cost_of_blocking_work`, because the cost belongs at the
call site and not in a comment. A test asserts every point defaults to open —
if that ever inverts, one broken integration stops every team.

**Registration is build-once-then-frozen.** `RegistryBuilder::build` consumes
the builder, so there is no path from a live `Registry` back to a mutable one: a
contribution cannot appear halfway through a render, and nothing holds a `&mut`
across an `await`. Per point the registry is bounded at 64, and the overflow
policy is named per [24](24-CONCURRENCY-AND-IDEMPOTENCY.md) §D-040 — the
newcomer is refused, because dropping an existing contribution to make room
silently disables a feature a workspace was relying on. A duplicate key is
refused for the same reason rather than overwriting.

**Still to come before C-017 is `Gated`:** the frontend half. C-018 renders the
drawer's panels and the palette's entries *from* this registry rather than from
a hard-coded list; until it does, the core exercises the contract in Rust only,
which proves the shape but not the rendering.

### D-043: not closed, and the honest answer

**The accepted direction was already tried, and it is not sufficient.** D-043
says "keep RLS; try a tenant-filtered projection first". The query C-013 emits
*is* that shape — `s.workspace_id` and `s.project_id = ANY(...)` applied to
`task_search` itself so `task_search_scope_ix` can combine with
`task_search_gin` — and it is the same shape
`tests/explain/queries/11` has been probing since Phase 0, where the measurement
that opened D-043 was taken.

The blocker is not the predicate. Under row-level security `@@` resolves to
`ts_match_vq`, which is not `LEAKPROOF`, so PostgreSQL will not evaluate it
before the row-security qual and **cannot use the GIN index as an index qual at
all**. No arrangement of tenant predicates changes that; the previously recorded
measurement is `Parallel Seq Scan` as `taskforge_app` against `Bitmap Index Scan`
as the owner, on the same 2M-row corpus, with RLS the only difference.

So, plainly: **`GET /api/v1/tasks?q=` is index-served at the gate's corpus and is
not expected to be at reference scale.** The `explain-no-seq-scan` job will pass
and does not prove the rule holds. Every remaining option — a `LEAKPROOF`
wrapper, a `SECURITY DEFINER` projection reader, dropping RLS on `task_search`
in favour of an explicit predicate, or ADR-014's external engine — trades
tenant-isolation guarantees against query plans, which is what D-043 already
identifies as an ADR decision touching ADR-011, ADR-014 and ADR-020. C-013 does
not settle it in an implementation.

**Still to come before C-013 is `Gated`:** D-043, and three of
[26](26-SEARCH-INDEXING-AND-QUERY.md) §Acceptance gates — the cursor property
test over random interleavings, the latency gate (which needs the reference
corpus and a reference machine, F-007), and an `EXPLAIN` case per sortable field
rather than per endpoint. The permission-filter gate that document names **is**
here: a task whose text matches harder than the visible one, in a project the
caller cannot see, and the assertion is that it never appears.

**Gaps this opened, reported rather than quietly absorbed:**

- **Symbol resolution uses UTC.** [27](27-FILTER-AND-SAVED-VIEW-DSL.md)
  §Timezone requires the actor's — "`due before @today` must mean the same thing
  to someone in Auckland and someone in Los Angeles" — and `user_account` has
  nowhere to store one. `resolve::Context` demands an offset, so UTC is passed
  and named here rather than a header being invented.
- **One sort key only.** [27](27-FILTER-AND-SAVED-VIEW-DSL.md) documents
  `sort=-due_at,key`; the cursor carries one key plus the id tiebreaker and the
  compiler emits one keyset comparison. A second key is **refused**, not
  silently dropped — honouring only the first would make the order
  non-deterministic across pages, which is the bug the mandatory tiebreaker
  exists to prevent.
- **`sort=status.position` cannot execute.** It maps to `ws.position`, which has
  no `FROM` entry; it needs a join to `workflow_status`. Pre-existing, now
  reachable from a URL, and refused at the edge.
- **No `IN`-list bound.** [26](26-SEARCH-INDEXING-AND-QUERY.md) §Query limits
  caps it at 100 and `casual-task-search::validate` counts clauses and depth
  only, so `?state=` with ten thousand values is accepted. Clause count, depth,
  page size and search-term length are all enforced.
- **An unknown symbol reports `TF-QRY-0003`.** [20](20-ERROR-CODE-REGISTRY.md)
  has no code for it; the operator/value code is the closest true statement, and
  a new one should be registered rather than guessed at here.

**One error code moved, and it is a contract change.** Before the grammar was
wired, every unknown query parameter on `GET /tasks` went through a generic
`reject_unknown` and reported `TF-VAL-0002`. The query string of that endpoint is
now the grammar plus its reserved pagination keys, so an unrecognised key is an
unknown *filter field* — `TF-QRY-0001`, which is what
[26](26-SEARCH-INDEXING-AND-QUERY.md) requires for an unlisted field. The status
stays `400`, so a client switching on status is unaffected; a client switching on
the code sees a different one. It was caught by a C-006 test that asserted the
old code, and it is written down here rather than absorbed into that test
silently.

**One pre-existing defect fixed in passing.** `archived` is `BOOLEAN` in the
grammar and a nullable timestamp in the schema, and compiled to
`'true'::timestamptz` — a filter that returned a 500 rather than an answer. It
now compares `archived_at IS NOT NULL`. It was unreachable before C-013 because
no caller could express it.
**C-016 is `Building`. Notifications exist, and the worker still cannot be
started.**

Before this, a task could be assigned, commented on and transitioned and nobody
was ever told. Now the fan-out turns an outbox event into an in-app record and,
for ranks 1–3, an email — through the `Mailer` C-001 already built.

Four modules, four reasons to change, none over 300 lines:
`casual-task-notification::{reason, audience, email}` are pure — no I/O, no SQL,
no clock — and `casual-task-persistence::{notification, audience}` holds every
statement. The consumer in `casual-task-worker::notify` composes them.

**Two schema gaps had to be closed before the headline rule was even
expressible.** `docs/29` rule 1 is "you are never notified about your own
action", and `outbox_event` carried **no actor** — so a consumer could not tell
who caused an event, and the one rule every tracker is complained about for
getting wrong was the one the schema made impossible. `docs/25`'s event envelope
had specified `actor` all along; migration 0024 stores it. `Claimed` carries it alongside the `workspace_id` and `project_id` C-015
added, because the dispatcher's role is granted on the two outbox tables and
nothing else (migration 0014, deliberately) and a consumer that reads assignees
needs a scope of its own.

**Two bugs in the mail path, both found by composing the first subject that
carries customer content:**

- `format_rfc5322` **refused** any non-ASCII subject, with a note that an RFC
  2047 encoder was a dependency the module deliberately did without. Correct
  while the only mail was a password reset with a fixed English subject. With
  `[WR-125] Task title`, every notification about a task titled `Café` would
  have failed to send, silently, per tenant. `casual-task-infra::header` is that
  encoder — 30 lines and no dependency, because widening `deny.toml` is the
  decision D-050 already refused.
- `Message`'s `Debug` printed the **subject**, and `LoggingMailer` logged it.
  Both were right until the subject became a task title, which `docs/46` forbids
  in a log line at any level. The file's own comment had predicted this — "when
  one is, the subject becomes tenant content and this impl is where that is
  noticed" — and this is that change.

**What is not in, and why it is a table rather than effort.** Preferences,
subscriptions (`SUBSCRIBED` and one-click unsubscribe), quiet hours and digests
all need tables `docs/29` assumes and migration 0008 does not provide. The
documented defaults are therefore the *whole* policy rather than its fallback —
narrower than the document, in the safe direction. **D-059**.

**C-016 cannot be `Gated` until the worker runs it.** `dispatch::claim` requires
a `BYPASSRLS` role and the consumer's reads require `taskforge_app` — two DSNs —
and `docs/48` names one `DATABASE_URL`. So `main` still starts nothing, and says
which decision is missing rather than restart-looping against a `verify` that
would refuse the application role. **D-060**. Ten integration tests drive the
consumer directly, which is exactly how the loop calls it; what is untested is
the process wiring, not the fan-out.

**One corpus defect, not fixed here.** `tests/explain/seed.sql` writes
`reason = 'ASSIGNEE'`, which is not one of the six reasons `docs/29` defines —
`ASSIGNED` is. It affects only the planning corpus, and changing seed data
changes the plans the `explain-no-seq-scan` gate compares against, so it belongs
in a change that can re-baseline that gate.

**D-060 is consumed, and the dispatch loop runs for the first time outside a
test.** The answer was already in the repository: `deploy/docker-compose.yml` has
set `DISPATCHER_DATABASE_URL` since it was written, and **nothing read it** — the
same failure mode `StorageConfig` was added for, which
[48](48-DEPLOYMENT-PROFILES.md) calls out as "configuration that is documented
and unread is worse than undocumented: it reports success". So no variable was
invented; the existing one is now declared in [48](48-DEPLOYMENT-PROFILES.md)
§Configuration and read by `Config`.

Two DSNs, because the two roles are deliberately different and neither can do the
other's job: `dispatch::claim` polls across every tenant and must bypass
row-level security (migration 0014, and `DispatcherRole::verify` refuses anything
else), while the consumers read tenant data and must **not**. A notification
fan-out reading assignees as the dispatcher would be reading them with RLS
switched off.

Both binaries now start it:

- **The API process** (Profile 1, `TF_WORKER_EMBEDDED` default true) spawns one
  loop per consumer on a small pool of its own, after verifying the role. Every
  refusal to start the loop is logged with what is off and what it costs —
  "notifications and live updates will not be delivered" — rather than leaving an
  operator to infer it from silence.
- **The worker binary** (Profile 2) requires both DSNs and **refuses to start**
  without them, per [48](48-DEPLOYMENT-PROFILES.md): "a misconfigured deployment
  must not start". It previously started successfully and did nothing.

**One thing that profile still cannot do, said plainly:** run SSE fan-out
usefully. A separate worker publishes into its own in-process hub, and the
browsers are connected to the API process. [48](48-DEPLOYMENT-PROFILES.md)
already requires Redis at more than one process for exactly this reason; the
worker now warns at startup rather than appearing to work.
### C-018 and C-019: the web client, and what it cannot do yet
**C-018 is `Building`. C-019 is `Built`.** `webapp/` stops being a bundle-floor
harness and becomes the product: sign-in, a workspace switcher, a board, a list,
My Work, a task drawer, and a command palette — all against the real API, with
no mock and no fixture anywhere in `src/` outside `src/floor/`.
**One transport carries every obligation.** `docs/05` §Authentication and
`docs/40` put four requirements on a browser call — the session cookie, the
double-submit CSRF token, the workspace header, and `If-Match`/`Idempotency-Key`
on writes. All four live in `api/http.ts`, and no view calls `fetch`. That is a
mechanism rather than a rule: a call site *cannot* forget the CSRF header
because it never sets one.
**Refusals are rendered from the registry, never from a body.** `docs/20`'s
codes map to a sentence and a remedy in `api/problem.ts`, and a test asserts the
server's own message never reaches the screen. `ApiError` also records whether a
response carried the error envelope at all — which is how a 404 from the router
(the route is not served) is told apart from a 404 from the application (absent
or invisible). The two look identical on the wire and mean opposite things to a
client.
**Affordances come from `/permissions/effective`, not from a role check.** A
`conditional` grant counts as permission: only the server can evaluate a
constraint for a given task, and hiding it would hide the reporter's own edit on
the task they reported — the exact case the constraint exists to allow. Hiding a
control stays presentation; the server re-authorizes every mutation.
**The drawer's panels and the palette's entries come from the extension point
registry** (`docs/34`, C-017), so the core's own contributions go through the
seam a plugin will. A registered contribution with nothing behind it renders
with its declared title and the reason rather than being skipped — skipping is
what makes a registry decorative.
**The registry table is mirrored in TypeScript, and that is a known defect.**
`crates/casual-task-plugin-contract/src/core_contributions.rs` has no HTTP
surface, so a browser cannot read it. `webapp/src/extensions/coreContributions.ts`
copies the rows and says so at the top. The two will drift the first time
someone edits one side; the fix is C-017's remaining half, an endpoint.
#### Workflow-backed board columns

`docs/23` fixes five permanent states, and every `TaskView` carries the one it is
in, derived from its status in the same statement so the two cannot disagree.
The browser now reads the workflow and renders its authored statuses as columns;
the permanent state remains the cross-workflow reporting and policy contract.
D-066, not an old board placeholder, tracks the remaining workflow-ownership
question.
#### The gaps that remain after the server and browser surfaces landed

Workflow reads, drag-and-drop, assignee reads, relations, activity, attachments
and live browser updates have all landed since the original C-018 audit. The
remaining gaps are narrower and retain their owning row:

| Not built | Why | Row |
| --- | --- | --- |
| Watched tasks in My Work | `watcher` is not in the filter grammar's closed field set and no endpoint exposes watchers | C-008 |
| Permission controls invalidated within a second | The server revalidates and closes a stream on an `authz_epoch` bump, but the browser reconnect does not invalidate its effective-permission query; controls can remain for the five-minute stale time while server authorization already refuses them | C-003 |
| Saved views, the filter builder, bulk operations, the rich-text editor, offline drafts and the retry queue | Not started | C-018 |
| Full design-system adoption | The dependency is consumed by newer surfaces, but the older local token and component layer has not been fully retired; bundle and geometry budgets must remain green during migration | C-018 |
**C-019 is `Built`, not `Gated`, and the reason is contrast.** Two layers run on
every push: `eslint` with `jsx-a11y` over the source, and `axe-core` over
rendered DOM under jsdom. jsdom has no layout, so axe's `color-contrast` rule
cannot run — it is disabled explicitly in the helper rather than left to report
"incomplete", which fails nothing and appears in no report. `docs/42` requires
4.5:1 verified in light and dark; that verification is **not** in this suite, nor
is focus order, nor anything measured. Those need the Playwright row
[15](15-CI-AND-RELEASE-GATES.md) still lists as open. A suite called "the
accessibility gate" that silently skips contrast is worse than no suite, because
it retires the question.
A third gate runs beside them and is not about accessibility: `boot.test.tsx`
mounts every route. `tsc` proves the types agree and proves nothing about a
provider in the wrong order or an import cycle — both of which are a blank page
that passes `typecheck` and `build`. `scripts/dev-up.sh` exists because "the
tests pass" and "you can open it" turned out to be different claims; this is the
cheap half of that lesson.
**C-010 is `Built`.** The whole handshake `docs/28` draws: pre-sign, direct
upload, commit, and a download that refuses anything the scanner has not
cleared.

**Files never pass through the API process.** `ObjectStore` has no method that
takes a body — the only way to write an object is for the client to `PUT` the
URL `presign_put` mints — and the only method that returns content returns a
bounded **prefix**, for the magic-byte sniff. A test asserts the absence of a
`put`, because the guarantee is the absence.

**The type is decided by the bytes, twice over.** `sniff` takes no declared type
at all, so it cannot be called with the client's; markup is checked *before*
signatures, because a GIF/HTML polyglot must be a refusal and a signature-first
sniffer stores it as an image; and commit separately rejects a declaration that
contradicts the bytes (`TF-ATT-0003`), which is the case `docs/28` §Validation
names. Anything unrecognised is `application/octet-stream`, which is inert.

**The invisibility invariant is structural, not remembered.** Every
client-facing read in the repository writes `committed_at IS NOT NULL`; the one
read that must see an uncommitted row is called `find_for_commit`; and
`mark_scanned` is the only statement that assigns `committed_at`. Two unit tests
count those occurrences in the source, because the invariant is a property of
*how many places* can write it.

**Two documented-but-unread configuration keys are now read.**
`TF_STORAGE_BACKEND` and `TF_STORAGE_PATH` appear in
[48](48-DEPLOYMENT-PROFILES.md), [52](52-DEPLOYMENT-GUIDE.md) and
`deploy/docker-compose.yml`, and nothing consumed them — so a deployment could
set `TF_STORAGE_BACKEND=s3`, have it accepted, and get local disk. `s3` is not
built, so it now refuses to start rather than pretending.

### D-062: the scan default, proposed and not settled

`docs/28` says "ClamAV by default; pluggable"; [48](48-DEPLOYMENT-PROFILES.md)
lists `scan` among the worker's jobs and gives it **no configuration key** and no
statement about a deployment without one. That is a real gap with two opposite
answers, and both are bad in different directions:

- **Fail closed** — no scanner means the attachment stays `PENDING` and is never
  downloadable. Uploads work and downloads do not, out of the box.
- **Fail open** — mark it `CLEAN` and serve it. The product's own stated
  invariant, that nothing is downloadable before it is scanned, becomes untrue
  and nothing says so.

C-010 implements **fail closed**, because it is the only default that does not
quietly make a security claim false, and because `AGENTS.md` puts correctness
above UX. It is recorded as **D-062 and flagged as needing the user's
countersign** rather than treated as settled: the cost — a single-node
deployment where every download 409s until an operator configures a scanner — is
a product decision, not an implementation detail.

The seam is in place: `guard::scanned_clean` matches on the verdict with no
catch-all arm, so a fifth scan state cannot be added without deciding whether it
may be served, and an unrecognised verdict is refused.

**Still to come before C-010 is `Gated`:** the orphan sweeper `docs/28` requires
in both directions, the S3 backend, inline preview, download auditing, and the
per-workspace size and storage quota. The scan consumer and its clean,
infected, unavailable and unconfigured outcomes are now tested against
PostgreSQL. The streaming test still needs a 2 GB fixture, the EICAR path needs
a live ClamAV daemon, and the orphan test needs the sweeper.

**One gap in the checksum check, stated.** `docs/28` §Validation requires the
client-supplied SHA-256 to match at commit. C-010 validates its *shape* and
compares the **size**, and does not recompute the digest: doing so means reading
the whole object, and `ObjectStore` deliberately exposes only a bounded prefix.
Verifying it needs either a digest from the storage backend — S3 returns one,
the filesystem backend would have to compute it during the upload handler — or a
streaming read outside the API process. Tracked here rather than left as a
passing test that checks nothing.

**The task drawer's two empty panels are filled.** Both were the same shape of
gap: the write side had existed for a while and nothing read it, so the data
accumulated where nobody could see it.

**`GET /api/v1/tasks/{id}/activity` (C-011).** Every change has written an
`activity_event` in the same transaction as the change since C-011 (ADR-006).
Nothing read them. It is keyset-paginated newest-first, and the cursor carries
`(occurred_at, id)` rather than an id alone — `activity_event` is **partitioned
by `occurred_at`** (migration 0007), so an id-only cursor could not be resumed
without searching every partition. Actor names are resolved once per page, not
once per row.

The permission is **`task.history.read`**, which
[25](25-EVENTS-OUTBOX-AND-AUDIT.md) §The three streams assigns explicitly —
activity is the user-facing stream and "must be readable by anyone who can see
the task", while `audit.read` governs the compliance stream with its IPs and
before/after. Gating this on `audit.read` would have hidden a user's own task
history behind an administrator's permission.

**`GET`/`POST /api/v1/tasks/{id}/dependencies` (C-008).** No endpoint read a
task's relations at all, so the Relations panel had nothing to call. The read
shape is a choice — [05](05-API-SPEC.md) specifies only the write — and it is
recorded there: two named lists, `blocked_by` and `blocks`, each carrying
`key`, `title` and `state`, unpaginated because
[21](21-API-LIMITS-AND-QUOTAS.md) bounds dependencies at 100 per task.

**The cycle check is one statement, not a check above one.** `insert` is a
single `INSERT ... SELECT ... WHERE NOT EXISTS (<recursive reachable>)` under a
transaction-scoped advisory lock on the workspace. There is no separate "is this
safe?" call to forget, and no window in which a concurrent request closes the
loop from the other side — which is exactly what
[24](24-CONCURRENCY-AND-IDEMPOTENCY.md) asks for and why the lock is there. The
walk is bounded to [21](21-API-LIMITS-AND-QUOTAS.md)'s 64 hops, and **the cost
is stated**: a cycle that closes only at hop 65 is not detected.

**A live defect found on the way, and fixed.** The filter compiler's
`is_blocked` clause selected `d.blocked_task_id` — a column `task_dependency`
has never had; migration 0005 names the two ends `from_task_id` and
`to_task_id`. Both `?is_blocked=true` and `?is_blocked=false` returned
`TF-SYS-0001`, and it held back the built-in **My Work · Blocked** view
[27](27-FILTER-AND-SAVED-VIEW-DSL.md) ships.

The direction is a domain call, not a guess: [03](03-DOMAIN-MODEL.md) says
"`from` blocks `to`" and gates a transition on an **incoming** `BLOCKS` edge, so
a task is blocked when it is the `to` end — which is also the direction
`task::unresolved_blockers` already read and what `task_dependency_rev_ix` on
`to_task_id` indexes.

Nothing caught it because the C-012 suite asserts the compiler's SQL *text*,
which a wrong column name satisfies perfectly, and the `EXPLAIN` catalogue has
no probe for that field — the same blind spot that hid the cursor cast bug in
C-006. The test added with the fix reads `migrations/0005` and asserts that
every `<alias>.<column>` the compiler emits is a column that migration
declares, so the next wrong name fails in CI rather than in production.

**The design spec's constraints are met.** A restricted edge is **returned**
with its identity withheld rather than filtered out — [03](03-DOMAIN-MODEL.md):
a blocking task "shows as 'restricted' if the viewer cannot see its project,
never as its title", and dropping the row would show a task as blocked by
nothing. A cycle refusal **names the loop** (`ONB-4 → API-2 → ONB-4`) in both
the message and `details.cycle`. And blocked-ness is on `TaskView` itself,
computed in the same query as the row: the board disables a drop target rather
than letting a card spring back, so it must know before the drag, and asking per
card would be the N+1 [04](04-RBAC-AND-AUTHORIZATION.md) §The list problem
exists to prevent. A bulk endpoint was the alternative and was rejected — it is
a second round trip that can disagree with the first.

**What is not covered:** the activity stream has no project-level feed
(`activity_project_ix` exists and nothing reads it); `is_blocked` still has no
`EXPLAIN` probe, so the fix above is covered by unit and integration tests
rather than by the plan gate; and the subtask rollup
and the `task.dependency.override` reason are **not** implemented here — the
override lives on the transition endpoint (C-007), and its required-reason field
is a change to that surface rather than to these two.

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

**C-038 removes the product's only unreachable state.** A person who signed in
and belonged to nothing was shown one sentence — "You are signed in but belong
to no workspace yet. Ask an owner for an invitation." — on a screen with no
control on it. `POST /api/v1/workspaces` existed the whole time and the client
never called it, so the first person in an organisation to sign up was told to
go and find someone else.

The first-run screen now offers both routes out, and states the second one
rather than implying it: an invitation adds you to *someone else's* workspace,
which is not the same as having one. "New workspace" is also reachable from the
switcher for anyone who already has one — hung off a button rather than the
`<select>`, because the switcher only renders as a select when there are two,
which would have hidden it from exactly the person most likely to want it.

The navigation is gone from that screen. This file's own header already said
rendering the frame "with no workspace produces a nav whose every link 404s",
and it did: eight destinations, every one scoped to a tenant that does not
exist yet.

**The slug is offered, not demanded.** Asking for a URL handle before the
workspace exists is asking someone to decide about a thing they have not made.
It is derived as the name is typed — "Acme, Inc." becomes `acme-inc` — and
stops following the name the moment it is edited by hand, tracked with a flag
rather than by comparing the two strings so that typing a slug which happens to
match its derived form cannot silently re-arm the derivation. Eleven cases
assert the suggestion is one the server would *accept*: a suggestion that fails
validation is a rejection the person did not earn. Verified by removing the
leading/trailing-separator strip, which fails six of them.

**C-039 began with a stylesheet collision nobody could see.** `app.css` defined
`.create` as a flex **row** — left over from an inline composer that no longer
exists, alongside a `.create__title` that no component references — and it
loaded after `patterns.css`, where the real create form is a flex *column*. So
the form rendered as a cramped line of controls with its labels centred against
its inputs, and nothing in either file looked wrong on its own.

With that deleted, the form was still the wrong shape:

- **The project was the first field**, defaulting to "Choose a project…", so on
  a list scoped to "All projects" a required decision was the price of typing a
  title. It is context, not a field you fill in order, and it now sits in the
  form's header defaulted to the scope you are already in.
- **Type and priority were behind a disclosure** and description was absent
  entirely, so writing down what a task *is* meant creating it and opening it
  again. All four are on the form, with description and due marked optional
  rather than every required field marked with an asterisk.
- **There was no way to file two.** "Create another" keeps the form open and
  keeps what describes the *batch* — project, type, priority — while clearing
  what describes the task.
- **⌘↵ / Ctrl+↵** submits from either text field. On the fields rather than the
  `<form>`, because a form is not an interactive element and `jsx-a11y` is right
  to say so; and Enter alone deliberately does *not* submit from the
  description, where a newline is what it means.

Assignee is absent on purpose: it is a separate endpoint, so offering it here
would make create non-atomic — a task that exists unassigned after the second
call fails is worse than one field's absence.

**The workspace moved to the header.** It is the outermost scope — the rail,
every route under it and every request carry it — and in the rail it read as one
more item *inside* the navigation, which inverts that. Creating one is a popover
anchored to a plus beside the name: spelled out as "New workspace" it sat at the
same weight as search, which is not what a once-a-year action deserves.

**C-040 is one missing HTTP method.** `docs/28`'s pipeline has been complete on
the server since C-010 — presign, a separate object origin, commit, scan,
download — and the client had none of it. `task/unbuilt.ts` said attachments
were unavailable and gave a reason that was *almost* right: it said the object
origin was served by nothing. It is served, by `objects::object_router` on its
own listener, and has been.

The real reason is the separation the module exists for. The application is one
origin and the attachment origin is deliberately another — that is the control
that stops a stored HTML file executing in the application's origin — so a
browser `PUT` carrying `Content-Type` is **not a simple request**. It is
preflighted, the router had no `OPTIONS` route, and the preflight got `405`.
Presign returned a URL and the browser refused to use it. The pipeline was
complete and unreachable, and the note explaining why named the wrong cause.

The preflight is served now, hand-written rather than by adding `tower-http`
for four headers, and scoped to the one origin `TF_PUBLIC_URL` names rather
than `*` — a presigned URL is already a narrow capability, but "narrow" and
"anyone who obtains it may spend it" are different properties. The headers are
on the real responses too, not only the preflight: a browser discards a
cross-origin response that lacks them, so an upload would have failed *after*
storing the bytes.

Proved end to end from a browser against the running stack: presign `201`,
`PUT` `200`, commit `202`.

**The scan consumer now exists.** With `TF_CLAMD_ADDR` configured it is the only
path that moves an upload from `PENDING` to `CLEAN`; without a scanner the file
remains invisible and undownloadable. That is D-062 working as countersigned —
fail closed — and the panel says so after an upload rather than showing
"Nothing attached" over a file that arrived safely.

**Also on the create form**, since a task you cannot attach to is half the
request: files are chosen before the task exists and uploaded after it, because
a presigned URL is minted per task. Sequentially, so ten files are ten uploads
and not thirty requests racing one rate limit.

**C-041 — the task drawer belonged to four views instead of to the address.**
`?task=` opened it on the board, the list and My work, and did nothing at all
on Home and Environments: each view rendered `<TaskPeek>` itself, and two of the
five surfaces that show task cards simply did not. Clicking a card there changed
the address and produced no drawer.

It is rendered once by the shell now. The drawer is a property of the URL —
every route already shares that parameter, and `.peek` is `position: fixed`, so
where in the tree it sat was never load-bearing. Four copies were four chances
to forget, and two of them had been taken.

Gated by an assertion over every card surface, verified by deleting the shell's
copy and watching it fail with `/home did not open the drawer`.

**C-042 is the consumer `docs/25` named and nobody wrote.** `docs/28` sets
`committed_at` on `PENDING → CLEAN` alone, and every read of an attachment
requires it — that is what makes a forgotten `WHERE` clause unable to leak an
unscanned file. The cost is that until something scans an upload, the file is
stored and invisible, and **nothing ever did**. C-040 made uploads reach
storage; they arrived somewhere nobody could look.

The consumer runs in the API's embedded dispatcher beside the other four, since
that is where the single-node profile runs them.

**`TF_CLAMD_ADDR` is the switch, and its absence is not a pass.** Unset, the
consumer acknowledges the delivery, logs why, and changes nothing — the file
stays `PENDING` and unreadable. That is D-062, countersigned, and the reason
the scanner is an `Option` rather than a default that waves files through: the
alternative default is a deployment that serves unscanned user content while
appearing to work.

A *failed* scan is not a verdict either. An unreachable daemon returns `Err`,
which leaves the delivery unacknowledged for the dispatcher to retry, rather
than recording an answer nobody gave.

ClamAV is in `deploy/docker-compose.yml` behind a `scanning` profile — the image
is ~1 GB and loads a signature database on start, which is not a cost to pay by
accident. Running the daemon and pointing the application at it are two separate
settings on purpose: an operator who does one gets a warning in the log rather
than silent non-scanning. Nothing was added to the dev script.

Four tests against a real database cover the four outcomes — clean commits,
infected never commits and its object is deleted, a failed scan changes nothing
and is retried, and no scanner is not a clean verdict. The last was verified by
making the mistake it forbids: teaching the consumer to mark unscanned files
`CLEAN` fails it and nothing else.

**C-043 adds the measure `docs/38` already specified.** `age` is
"`created_at` → now, for open tasks", and the dashboard was missing "oldest open
work" because of its absence. Two properties are in the compiler rather than
left to the caller:

- **It does not join `task_state_interval`.** Age is bounded by the clock, not
  by a transition, so joining the projection would drop a task created a second
  ago — one with no intervals yet — out of a measure specifically about work
  sitting untouched.
- **Completed and cancelled work is excluded in the SQL.** The age of a finished
  task is not a smaller number, it is a meaningless one: it keeps growing after
  the work stopped. Leaving that to a filter would make the measure mean
  different things depending on who wrote the filter.

It defaults to `max`, not a percentile: "how old is the work" is asked as "what
has been sitting longest", and a median hides the one task the question is
about.

**Adding one tile then broke the shell containment**, which is worth recording
because it is the same defect twice. Every chart carries its numbers in a
`position: absolute` `.visually-hidden` table, and a tile that is not a link had
no positioned ancestor — so those boxes were laid out against the initial
containing block, escaped the scrolling region, and stretched the *document*.
One clipped pixel, at whatever offset the chart sits at, and the whole
application scrolled again on that one route.

The horizontal version of this was the hidden table making the page 62 px wider
than the viewport (C-035). `position: relative` on `.tile` closes both, and the
assertion that caught it is the one that injects its own height — verified by
deleting the line and watching it fail.

**C-044, part one: a dragged card went behind everything.** Reported directly.
`BoardCard` translated the card in place — it stayed a child of `.column__body`,
which is a scrolling region, so it was **clipped by its own column** and painted
under every column to its right. No `z-index` fixes that: an element cannot
escape an ancestor's `overflow` clip whatever it is stacked at, which is why
the obvious fix would not have worked.

`DragOverlay` renders the moving card outside that subtree, which is the only
thing that can follow a pointer across columns. The original stays in place at
40% to mark the gap it came from, and the task travels in dnd-kit's `data` so
the overlay — drawn by the context, which never holds the task list — can render
it. Verified in the running application: with a card picked up,
`.card__dragging` exists and `closest('.column__body')` is `null`.

**Part two: `created_vs_completed`**, the last measure `docs/38`'s closed set
specified that had no implementation, and the last dashboard tile documented as
missing. One query, not two runs: the whole message is where the lines *cross*,
and two runs would be two permission resolutions and two cache windows, so the
crossing point — the only thing anyone reads it for — is exactly where that
error would show.

It takes no dimension. The two series *are* the grouping, so the response says
`"group_by": "series"` rather than echoing a dimension the answer does not
contain. `LineChart` draws several series on one scale — independent axes can be
made to cross anywhere — and the second is dashed as well as differently
coloured, because two lines told apart by hue alone are two lines some readers
cannot tell apart.

**The fixture was the reason there was no test.** `/projects/{id}` answered a
`{data: […]}` page where a single project belongs, so `workflow_id` was
`undefined`, the board never resolved a workflow, and it rendered **no drag
handles at all** — a drag test would have exercised nothing and passed. A
fixture that answers the wrong shape does not fail a test; it quietly removes
what the test was going to look at.

Corrected, with a workflow of two statuses and the `task.transition` grant the
handles need. The two failures noted when this was first tried were my own
mismatched ids in the fixture, not a product defect: with the ids consistent
the suite is green, and the drag now has the regression test it should have had
— asserting the *mechanism* (the moving card is not inside a scrolling column)
rather than pixels, and verified by restoring the in-place transform and
watching it fail.

**C-045 stops the product drawing one number two ways.** Reports rendered a
`<div>` with a width per row and called it a bar. That was honest while there
was nothing better, and stopped being honest the moment C-035 gave the dashboard
a chart set — two surfaces answering the same question with two different
pictures is what a design system exists to prevent.

It uses `BarChart` now, and the table lost its own bar column: with the chart
above it there were literally two bar charts on the page. What is left is the
division the chart set already assumes — the drawing carries the shape, the
table carries the numbers, right-aligned on tabular figures so a column of
counts lines up on its digits. Both are bounded to the same width so they read
as one block.

The chart's stylesheet is imported by `ReportsView` as well as the dashboard: a
component that only looks right on one route has a hidden dependency on that
route's stylesheet, which is a trap for whoever uses it third.

**C-046 closes `docs/38`'s measure set.** `time_in_state` had been refused by
name since the set was written, for a reason the refusal itself stated: it needs
a state named and the request had nowhere to say which. That was a missing
*field*, not a missing decision — the measure is in the closed set — so `state`
is now a field of its own rather than a suffix on the measure name
(`time_in_state_active`), because the states are data that `docs/23` owns while
measure names are a vocabulary this module parses. Folding one into the other
would make every new state a new measure name nobody registered.

Four properties are enforced rather than assumed:

- **Total per task, not per visit.** A task can enter a state several times, and
  "how long was this in review" means all of it. Reducing the intervals directly
  would answer "how long was a typical visit", which flatters a task that
  bounced five times into looking quick five times over.
- **An open interval counts up to now.** The task sitting in a state right now
  is the one the question is usually about; ignoring it would report the state's
  cost as zero for exactly the work stuck in it.
- **A permanent state, not a status.** A status is named inside one project's
  workflow, so at workspace scope two projects can both have a "Review" and the
  answer could not say which it meant.
- **`state` is refused for every other measure.** A parameter the answer ignores
  is one a caller will believe narrowed their report.

It is deliberately **not** in the client's `MEASURES`. That list is what the
Reports toolbar offers, and the toolbar has no control for naming a state — so
offering it there would be a menu entry that always produced a `400`. Dashboards
use it through a tile, which names the state in its own definition.

The tile that uses it is titled "Time in progress" and not "Time in Blocked",
which is what I first wrote: `ACTIVE` is a permanent state and the default
workflow maps *two* statuses onto it, so the number covers In Progress and
Blocked together. A title naming one column would have been a wrong number with
a confident label.

**C-047 is the gap between "it works" and "somebody else can run it".** Four
things were missing at once and they are one item because they fail together: a
`README` that still said there was no user interface and none was being built, a
`docker-compose.yml` that could only pull a published image and never build the
repository it sat in, an environment reference that did not mention the scanner
address without which every attachment is invisible, and no public page at all.

`build:` is declared beside `image:` rather than replacing it, so a plain `up`
still pulls and `up --build` compiles. Somebody evaluating the product wants the
image; somebody changing the code wants their own build, and making the second
the default would put a Rust toolchain in the path of the first.

The site is one hand-written HTML file with an inline stylesheet and no
JavaScript, published by `.github/workflows/pages.yml` on changes under `site/`.
No static site generator: a toolchain to maintain for a single document is a
cost with no return, and the page is fast because there is nothing to load.

Its palette is the product's own — the forge orange `#d8610b`, and `#f08a2e`
where a dark background needs the lighter one — and the mark is
`webapp/public/brand/taskforge-mark.svg` inlined so it inherits the text colour.
The first draft used a teal accent and no mark at all, which taught a visitor
the wrong thing about the product before they clicked through.

The three screenshots are captured from a running build rather than drawn, and
`og:image` points at the dashboard one. A marketing page for a product with a
user interface that shows none of it is asking to be taken on trust, and this
project's whole argument is that you should not have to.

Three things carry the optimisation, and only one of them is conventional SEO:

- **Search** — a title and description made of the words somebody would type
  ("self-hosted", "work tracker", "Rust", "Apache-2.0") rather than a slogan, a
  canonical URL, Open Graph tags, and a sitemap.
- **Answer engines** — JSON-LD `SoftwareApplication` and `FAQPage`, so the
  questions a machine is asked ("is it production ready", "what does it cost")
  have answers on the page in the form that gets quoted.
- **Models reading the repository** — `site/llms.txt`, which states in prose
  what is built and what is designed-and-not-built.

All three say the same thing, and what they say is that this is a Phase 1 core
and not a finished product. That is the part worth defending: structured data
is the half a machine trusts without reading the prose around it, so a
`FAQPage` claiming production readiness would be the most load-bearing lie on
the site. The status paragraph sits above the fold for the same reason.

`twitter:card` is `summary`, not `summary_large_image`: there is no card image
yet, and declaring one that does not exist renders a broken card rather than a
small one.

The workflow does **not** enable Pages itself. `actions/configure-pages` has an
`enablement: true` flag that reads as though it would, and the first deploy
proved it cannot: creating a Pages site needs admin rights `GITHUB_TOKEN` does
not have, so the flag fails the run outright and succeeds only once Pages
already exists — only, that is, when it has nothing to do. Pages was enabled
once with `gh api -X POST .../pages -f build_type=workflow`, and the workflow
says so where somebody forking this will read it.

It is a separate workflow from `ci.yml` because it deploys — `ci.yml` is `contents: read` by
design, and a deploy job inside it would either hold a release behind a static
page or publish one from a commit the gates rejected.

The deployment claim is now bounded by a release preflight. It refuses to
publish while D-048 is unresolved, a Dockerfile base is mutable, or the
existing-volume upgrade runner is absent. The single-node first-start path is
still `Built`; production release remains blocked rather than inferred from a
green clean-database compose test. [18](18-SUPPORT-MATRIX.md),
[48](48-DEPLOYMENT-PROFILES.md), and [52](52-DEPLOYMENT-GUIDE.md) state the same
boundary.


**C-048 is the third of a promise the shell has always made.** The search button
reads "Search tasks, projects and people". The palette searched tasks and
projects; nothing ever fetched people.

What made it look broken rather than merely incomplete is the search projection.
Weight B indexes the *reporter's* display name on every task, so typing a
colleague's name returned every task that person had ever raised — in a young
workspace, all of them — ranked by relevance, with nothing on the row to say
why. Eight results that each look arbitrary read as a bug, not as a feature
answering a different question.

Members now become commands beside the workspaces, filtered on the client for
the same reason projects are: membership is a small, slow-changing list, and a
query per keystroke buys nothing. Choosing one sets `?assignee=`, which is a
real answer to "what is this person working on" — deliberately not a link to a
profile page, because there is no profile route and a command that navigated
nowhere would be the lie `registry.ts` was written to avoid.

The email is a keyword and not part of the title: it is how you find a colleague
whose name you cannot spell, and it is not something to paint across a list that
may be on a shared screen. It is `null` once an account is anonymized (ADR-026),
which is the moment it must not appear — interpolated straight in, that becomes
the literal string "null" as a searchable term matching every anonymized person
at once. A test asserts it, and the test was confirmed by making the mistake.

**What this does not fix, and why it was not fixed here.** The search sweep found
two more defects, both registered rather than patched: the trigram index that is
written on every task and read by nothing (**D-069**), and the missing recency
decay (**D-070**). Both look like small changes and neither is: the first is a
query-plan change that `compile_search` itself warns about under D-043, and the
second alters the expression that is simultaneously the sort key and the keyset
cursor. Guessing at either would have been the kind of silent resolution this
tracker exists to prevent.

**C-049 is the other half of C-048's finding.** The projection indexes four
bands (`docs/26` §Weighting) and the palette row showed one of them. A task
matched on its description, or on the name of the person who raised it, arrived
looking like a task that matched on nothing — which is how a search that is
working correctly becomes indistinguishable from a broken one. The same query
that returned eight arbitrary rows now reads "assigned to Demo User", "reported
by Demo User", against each.

Inferred on the client, not asked of the server. PostgreSQL would answer
precisely with `ts_headline` — it knows which lexeme matched and this does not —
at the cost of a second pass over every document on every keystroke and a change
to the wire type. The row already carries `title`, `description`, `reporter_id`
and `assignees`, and the palette already holds the members for C-048, so the
common cases are named with no request at all.

The consequence is stated rather than hidden: this can be **wrong about the
reason** while the match is right, because a stemmed hit or one in a comment or
tag body is not on the row. So the fallback says where it did *not* match —
"matches elsewhere in the task" — instead of guessing a place. A confident wrong
answer is worse here than a vague right one, and the note is a subtitle, never
load-bearing.

Nothing is annotated when the title already contains the query. A note on every
row is a note nobody reads, which would cost the one case this exists for.
