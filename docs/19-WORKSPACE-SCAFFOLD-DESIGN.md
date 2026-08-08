# 19 — Workspace Scaffold & Layer Division

The **layer-division contract.** Crate boundaries and the dependency DAG are
fixed before code so no layer needs a do-over when plugins (Phase 3) or
automations (Phase 4) land. Changing a boundary requires a new ADR (ADR-003).

The guiding rule: **a lower layer never knows about a higher one, and
authorization is a layer everything consults but nothing embeds.**

## The dependency DAG

```
                          casual-task-api  ◀── the deployable binary
                                 │
                          casual-task-app   (command/query handlers, one txn each)
                                 │
        ┌────────────┬───────────┼───────────┬─────────────┬──────────────┐
        │            │           │           │             │              │
   task       workflow      project     identity      activity      attachment
        │            │           │           │             │              │
        └────────────┴─────┬─────┴───────────┴─────────────┴──────────────┘
                           │
                   casual-task-authz          ◀── consulted by app, never by domain
                           │
                   casual-task-model          ◀── the bedrock: types, IDs, errors
                           ▲
                           │
   casual-task-persistence ─┘   (SQLx repositories; implements traits from domain)
                           │
   casual-task-infra ──────┘   (redis, object store, mail — all behind traits)

   casual-task-worker  ◀── separate binary; depends on app + infra + plugin-contract
   casual-task-plugin-contract  ◀── the extension point registry & manifest types
   casual-task-search  ◀── projection build + query construction
   casual-task-observability   ◀── tracing, metrics, correlation ids
```

Read it as: **`casual-task-model` is the bedrock and depends on nothing.**
Everything points downward. `casual-task-authz` sits directly above the model so
every domain crate *can* be authorized without any domain crate *containing*
authorization.

## Crate-by-crate: what it owns, what it must never own

### Bedrock

**`casual-task-model`** — the shared vocabulary: ID newtypes (`TaskId`,
`WorkspaceId`, …), the `WorkspaceScope` capability token, `task_state` and the
other closed enums, the error type and code registry ([20](20-ERROR-CODE-REGISTRY.md)),
pagination cursors, and the `Permission` string registry. Depends on nothing but
`serde`/`uuid`/`time`.
Must **not** own: any SQL, any HTTP, any business rule.

> **Why `WorkspaceScope` lives here:** it is a type whose constructor is
> `pub(crate)` plus one `from_auth_context` entry point. Every repository method
> takes it. That makes "forgot the tenant filter" a compile error rather than a
> code-review responsibility ([32](32-TENANCY-AND-ISOLATION.md)).

### Authorization

**`casual-task-authz`** — the resolver from [04](04-RBAC-AND-AUTHORIZATION.md):
grant collection, principal expansion, scope-chain walk, constraint evaluation,
the `authz_epoch` cache, the grant/scope ceilings, and `explain()`.
Depends on `-model` and a `GrantStore` trait (implemented by `-persistence`).
Must **not** own: HTTP, or knowledge of what a task *is* beyond its scope chain.

Isolating it this way is what makes the matrix and escalation test suites
possible without a database or a web server.

### Domain modules

Each owns its aggregate, its invariants, and its repository *trait* — never the
SQL that implements it.

| Crate | Owns |
| --- | --- |
| `casual-task-identity` | users, workspace membership, teams, sessions, service accounts, API tokens |
| `casual-task-project` | projects, project membership, environments, milestones, tags |
| `casual-task-workflow` | workflows, statuses, transitions, the state mapping, transition validation |
| `casual-task-task` | tasks, assignees, dependencies (incl. cycle check), subtasks, ranks |
| `casual-task-activity` | activity + audit record construction, the outbox event shape |
| `casual-task-attachment` | attachment lifecycle, the scan/commit handshake |
| `casual-task-notification` | notification construction and preference evaluation |

**The rule that makes this real:** a domain crate does not depend on another
domain crate. `casual-task-task` cannot `use casual_task_project::...`. Where it
needs project data, it declares what it needs as a trait and `-app` supplies it.
An illegal dependency is a build failure, not a review comment.

### Application

**`casual-task-app`** — command and query handlers. Owns transaction boundaries,
cross-module orchestration, and the rule that **one command = one transaction =
one activity record = one outbox event**. This is the only layer permitted to
compose multiple domain crates.
Must **not** own: HTTP types, SQL, or domain rules that belong in a domain crate.

### Persistence & infrastructure

**`casual-task-persistence`** — SQLx implementations of every repository trait.
Owns connection pooling, the RLS session variable, `sqlx::query!` compile-checked
statements, and migrations. Owns **all** SQL in the system.

**`casual-task-infra`** — Redis, object storage, and mail, each behind a trait
with an in-process fallback so the single-node profile needs none of them
([48](48-DEPLOYMENT-PROFILES.md)).

**`casual-task-search`** — projection document construction and query building
([26](26-SEARCH-INDEXING-AND-QUERY.md)). Separate from `-persistence` because it
is the seam an external engine would replace (ADR-014).

### Edge & bridges

**`casual-task-api`** — the deployable binary. Axum routers, tower middleware
(auth context, request id, rate limit, timeout, compression, tracing), DTOs, the
OpenAPI document, SSE streams, and error → HTTP mapping. The **only** crate that
knows about HTTP.

**`casual-task-worker`** — the second binary: outbox dispatch, search projection,
notification fan-out, webhook delivery, scan coordination, automation execution,
retention sweeps, rank compaction.

**`casual-task-plugin-contract`** — extension point definitions, manifest types
and validation, scope registry, signing/verification. Depended on by `-api`,
`-worker`, and by external plugin SDKs. Versioned independently (ADR-015).

**`casual-task-observability`** — tracing subscriber, metrics registry,
correlation id propagation. Depended on by both binaries.

## Boundary invariants (the rules that prevent do-overs)

1. **No domain crate depends on another domain crate.** Cross-module needs go
   through traits satisfied by `-app`.
2. **All SQL lives in `-persistence`.** Any `sqlx::query!` outside it fails the
   lint gate.
3. **All HTTP lives in `-api`.** No domain crate names a status code.
4. **Every repository method takes a `WorkspaceScope`.** Minted only from an
   authenticated context (ADR-020).
5. **Authorization is consulted, never embedded.** `-authz` has no domain
   dependency, so it is testable in isolation and cannot be bypassed by adding a
   domain method.
6. **The outbox write is in the same transaction as the mutation.** Enforced by
   the handler signature: a command returns `(Change, Vec<Event>)` and `-app`
   commits both, so a handler *cannot* emit an event outside the transaction
   (ADR-006).
7. **No customer code in either binary.** The plugin contract crate defines
   types and transport, never an execution host (ADR-016).
8. **`unsafe_code = "forbid"`** at the workspace root; an exception needs an ADR.

## Repository layout

```
tasks/
├── Cargo.toml                    # workspace manifest
├── rust-toolchain.toml           # pinned toolchain
├── deny.toml                     # cargo-deny: licenses, advisories, bans
├── crates/
│   ├── casual-task-model/
│   ├── casual-task-authz/
│   ├── casual-task-identity/
│   ├── casual-task-project/
│   ├── casual-task-workflow/
│   ├── casual-task-task/
│   ├── casual-task-activity/
│   ├── casual-task-attachment/
│   ├── casual-task-notification/
│   ├── casual-task-app/
│   ├── casual-task-persistence/
│   ├── casual-task-search/
│   ├── casual-task-infra/
│   ├── casual-task-plugin-contract/
│   ├── casual-task-observability/
│   ├── casual-task-api/           # binary
│   └── casual-task-worker/        # binary
├── migrations/                   # versioned SQL, sqlx migrate
├── webapp/                       # React client (its own toolchain)
├── tools/
│   ├── casual-task-seed/         # deterministic reference corpus
│   └── casual-task-loadtest/     # latency gate harness
├── fixtures/                     # golden permission matrices, event samples
├── benchmarks/                   # committed named-environment baselines
├── fuzz/                         # separate workspace: filter grammar, manifest parser
├── docs/                         # this design record
└── docker-compose.yml            # dev profile
```

Mirrors `opendoc/` and `sheets/` so the three services feel like siblings.

## Workspace-level policy

- Rust 2024 edition, `resolver = "3"`, `unsafe_code = "forbid"`.
- Pinned toolchain; MSRV declared by ADR at Phase 0.
- Release profile: `lto = "thin"`, `codegen-units = 1`, `strip = "symbols"`,
  separate debug artifacts retained for diagnostics.
- `panic = "abort"` in release for the worker; **not** for the API — a panic in
  one request handler must not take down the process serving others.
- Runtime image: distroless or minimal Debian. Not `scratch` — TLS roots,
  timezone data, and the ability to exec a debugger in an incident are worth the
  megabytes.
- `cargo-deny` gates licenses (Apache-2.0 compatible only), advisories, and
  duplicate dependency versions.

## Test topology

| Layer | How it is tested |
| --- | --- |
| `-model`, `-authz` | pure unit + property tests, no I/O. The permission matrix is a golden fixture. |
| domain crates | unit tests against in-memory trait fakes |
| `-persistence` | `testcontainers-rs` against real PostgreSQL — never a mock, because the SQL *is* the thing under test |
| `-app` | integration tests with a real database, asserting transactional atomicity (change + activity + outbox together or not at all) |
| `-api` | HTTP-level tests including authorization, error codes, and cursor behaviour |
| cross-cutting | the `EXPLAIN` no-seq-scan suite and the latency gates against the seeded corpus ([26](26-SEARCH-INDEXING-AND-QUERY.md)) |

`cargo-nextest` is the runner. Database tests run against a per-test transaction
that rolls back, except migration tests, which need real DDL.
