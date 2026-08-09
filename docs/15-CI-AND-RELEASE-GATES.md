# 15 — CI & Release Gates

The PR contract. A gate that can be skipped is not a gate — everything below
blocks merge, and disabling one requires an ADR.

## Per-PR gates

### Build & style

| Gate | Command | Blocks |
| --- | --- | --- |
| Format | `cargo fmt --check` | ✅ |
| Lint | `cargo clippy --all-targets -- -D warnings` | ✅ |
| Build | `cargo build --workspace --all-features` | ✅ |
| Docs build | `cargo doc --no-deps` | ✅ |
| Dependencies | `cargo deny check` (licenses, advisories, bans, sources) | ✅ |
| Frontend lint/types | `eslint`, `tsc --noEmit` | ✅ |

### Correctness

| Gate | What | Blocks |
| --- | --- | --- |
| Unit + property | `cargo nextest run` | ✅ |
| Integration | testcontainers PostgreSQL | ✅ |
| **Permission matrix** | golden fixture: permission × role × scope | ✅ |
| **Escalation suite** | one test per control in [04](04-RBAC-AND-AUTHORIZATION.md) | ✅ |
| **Cross-tenant suite** | every endpoint, generated from the route table | ✅ |
| Migration | apply to seeded prior version; timing budget | ✅ |
| Frontend | Vitest + Testing Library | ✅ |
| E2E | Playwright core flows | ✅ |

### Contracts

| Gate | What | Blocks |
| --- | --- | --- |
| OpenAPI diff | vs committed snapshot; breaking change requires a version bump | ✅ |
| Event schema diff | payload changes require a `schema_version` bump | ✅ |
| Plugin contract diff | semver-checked | ✅ |
| Error registry | every emitted code exists in [20](20-ERROR-CODE-REGISTRY.md) | ✅ |

### Performance

| Gate | What | Blocks |
| --- | --- | --- |
| **`EXPLAIN` no-seq-scan** | every endpoint × sortable field, reference corpus | ✅ |
| Query count | no N+1; one authorization resolution per list | ✅ |
| **Bundle size** | shell ≤ ADR-024 budget | ✅ |
| Latency (subset) | reduced corpus, >10% regression vs baseline | ✅ |
| Latency (full) | full reference corpus | nightly |

### Schema & deployment

| Gate | What | Blocks |
| --- | --- | --- |
| **Schema verification** | every migration applied to a clean PostgreSQL 16; 8 structural assertions (every tenant table has `workspace_id`; every such table has a **FORCEd** RLS policy; no policy casts `current_setting` without `NULLIF`; the [26](26-SEARCH-INDEXING-AND-QUERY.md) index inventory exists; the five states are unchanged; the app role is not a superuser) | ✅ |
| **Tenant isolation, behavioural** | run as `taskforge_app`: unscoped sees nothing, scoped sees only its tenant, no pool bleed after COMMIT, no cross-tenant row | ✅ |
| **Append-only history** | `UPDATE`/`DELETE` on `activity_event` and `audit_event` are rejected | ✅ |
| **Image build** | multi-stage build; both binaries run; runs as uid 65532; migrations shipped | ✅ |
| **Image size** | under 100 MB ([30](30-PERFORMANCE-AND-CAPACITY-TARGETS.md)) | ✅ |
| **Deployment end-to-end** | the *deployment* compose comes up, creates a correctly-constrained role, and is verifiably isolated ([52](52-DEPLOYMENT-GUIDE.md)) | ✅ |

> Why the last one is not redundant with the schema gate: the dangerous
> deployment failures are **silent**. A missing environment variable makes the
> role-init script fail, PostgreSQL leaves a data directory behind, and the next
> start comes up *healthy with no application role* — at which point the
> application connects as the owner and row-level security is inert. Nothing
> looks broken. This gate exists because that bug was found by running the
> compose, not by reading it.

### Security

| Gate | What | Blocks |
| --- | --- | --- |
| Secret scan | no credentials in the diff (incl. `tf_pat_`/`tf_sat_` prefixes) | ✅ |
| SAST | `cargo audit` + semgrep rules | ✅ |
| Container scan | base image CVEs | ✅ |
| Enumeration test | login/reset/invite responses indistinguishable | ✅ |
| Injection property test | filter compiler emits no user-derived SQL strings | ✅ |
| Fuzz (smoke) | filter grammar + manifest parser, short budget | ✅ |
| Fuzz (deep) | extended budget | nightly |

### Accessibility

| Gate | What | Blocks |
| --- | --- | --- |
| axe automated | core flows, no violations | ✅ |
| Contrast | design system tokens, light + dark | ✅ |
| Keyboard-only | manual, per release | release |

## Custom lints

Enforcing the architecture, not just style. Each corresponds to an invariant in
[19](19-WORKSPACE-SCAFFOLD-DESIGN.md):

Run by the `architecture` job over `crates/` **and** `tools/`.

| Lint | Rule | State |
| --- | --- | --- |
| `no-sql-outside-persistence` | `sqlx::query*` only in `casual-task-persistence` | enforced |
| `no-http-outside-api` | HTTP types only in `casual-task-api` | enforced |
| `no-cross-domain-dep` | no domain crate depends on another domain crate | enforced |
| `scope-required` | every repository method takes a `WorkspaceScope` | enforced |
| `auth-context-at-edge` | only `casual-task-api` may mint an `AuthContext` | enforced |
| `no-offset` | `OFFSET` is banned in application SQL | enforced |
| `bounded-channels` | no unbounded channel constructor | enforced — see below |
| `no-io-in-transaction` | no HTTP/object-store client reachable from a transaction scope | **not built** — needs a transaction type to scope against (C-011) |
| `cache-key-scoped` | every cache key constructor takes a `WorkspaceScope` | **not built** — there is no cache until C-003 |
| `event-in-transaction` | handlers return events; they cannot publish directly | **not built** — needs the command layer (C-011) |

The last three are listed because they are part of the design, and marked
because a table that reads as nine enforced rules when six exist is exactly the
kind of claim this document is supposed to make impossible.

**`bounded-channels` is enforced twice, and the text lint is the weaker half.**
Matching source text cannot see through an alias, a re-export, or a call split
across two lines. The real gate is `clippy.toml`'s `disallowed-methods` and
`disallowed-types`, which resolve paths after name resolution and fail the build
with the reason attached — covering tokio, futures, crossbeam, flume,
async-channel, and `std::sync::mpsc::channel`, which is unbounded by definition
and contains the word "unbounded" nowhere. The same file bans `std::thread::sleep`
and `Runtime::block_on` for the blocking-in-async anti-pattern in
[30](30-PERFORMANCE-AND-CAPACITY-TARGETS.md). Most of those paths do not resolve
yet, because no async runtime is a dependency at Phase 0; clippy ignores an
unresolved path, so each entry begins enforcing the moment its crate is added —
which is the only moment it could still be added cheaply.

These are the difference between an architecture document and an architecture.

## Documentation gates

| Gate | Blocks |
| --- | --- |
| Every new/changed design decision has a numbered doc updated | ✅ |
| ADR-triggering change has an Accepted ADR | ✅ |
| Every new error code is in [20](20-ERROR-CODE-REGISTRY.md) | ✅ |
| Every new filter field is in [26](26-SEARCH-INDEXING-AND-QUERY.md) **with an index and an `EXPLAIN` assertion** | ✅ |
| Tracker row added or moved ([14](14-EXECUTION-TRACKER.md)) | ✅ |
| No broken internal doc links | ✅ |

The filter-field gate is the one that keeps the index contract honest over time:
you cannot add a way to query without adding the index that serves it, in the
same PR.

## Release gates

Beyond per-PR, before a version ships:

- Full latency suite at reference capacity, within targets.
- Deep fuzz run, clean.
- Manual keyboard-only accessibility pass.
- **Restore drill**: backup restored into a scratch environment, timed, verified.
- **Published artifact verified**: the image that was *pushed* — not the one
  built locally — is deployed via the deployment compose and asserted secure.
  A release that passes CI and then fails on a self-hoster's machine is exactly
  what this catches.
- **Multi-arch**: `linux/amd64` and `linux/arm64`. ARM is not optional — self-
  hosters run this on Apple silicon and on ARM VPS instances.
- **Provenance attestation** pushed to the registry alongside the image.
- Migration rehearsal against a production-shaped snapshot, timed.
- SBOM generated; release artifacts signed.
- CHANGELOG updated; breaking changes called out explicitly.
- Rollback verified to the previous version.
- Support matrix ([18](18-SUPPORT-MATRIX.md)) updated.

## Baselines

Performance gates compare against **committed baselines per named environment**
(`benchmarks/`), and fail on relative regression (>10%), not absolute thresholds.
Absolute thresholds fail on CI noise, get muted, and then protect nothing.

Updating a baseline requires the PR to state why the regression is acceptable —
which makes a deliberate trade visible and an accidental one obvious.

## Pending gates

The tables above are the **contract**: every row marked ✅ blocks merge once its
harness exists. Some do not exist yet, and a ✅ beside a gate nobody runs is the
single most misleading thing this document could contain. So they are listed
here, with the tracker item that lands each one. `.github/workflows/ci.yml`
points at this section, and the rule is: a gate is either a job in that file or
a row in this table. Never neither.

| Gate (from the tables above) | Lands with | Why not yet |
| --- | --- | --- |
| Schema gate asserts the auth `SECURITY DEFINER` definition | **C-001** | ADR-032 accepts the pre-workspace seam **on this condition**. The F-015 gate checks tables; a redefinition widening the function's `RETURNS TABLE` would pass today, which is exactly how a narrowed hole becomes a wide one. |
| Schema gate asserts the auth `SECURITY DEFINER` definition | **C-001** | ADR-032 accepts the pre-workspace seam **on this condition**. The F-015 gate checks tables; a redefinition widening the function's `RETURNS TABLE` would pass today, which is exactly how a narrowed hole becomes a wide one. |
| Latency (subset + full) | **F-007** | The harness and the comparison gate are built and tested. There is no baseline to compare against: `benchmarks/reference-8vcpu-32gb.reference.json` is a placeholder that no run can pass, because the docs/30 reference machine does not exist yet. |
| Frontend lint (`eslint`) | **C-019** | `tsc --noEmit` runs today in `bundle-size`. There is no ESLint config, and no product code to lint. |
| Frontend tests (Vitest), E2E (Playwright) | **C-018**, **C-019** | No product frontend exists; `webapp/` is the bundle-floor harness only. |
| ~~Integration (testcontainers)~~ | — | **Built (F-005).** `crates/casual-task-persistence/tests/schema_harness.rs` starts PostgreSQL 16, applies every migration, and reaches the invariants from Rust. Run by the `schema` job. The tests are `#[ignore]` so `cargo test` stays runnable without a Docker daemon; CI runs them explicitly, because otherwise nothing would. |
| Query count (no N+1) | **C-012** | Needs a query layer to count. |
| Permission matrix, escalation, cross-tenant | **C-004**, **C-005** | Phase 1. |
| OpenAPI diff, event schema diff, plugin contract diff, error registry | Phase 1 | Nothing to diff until `/v1` and the registry exist. |
| Secret scan, SAST, container scan, enumeration, injection, fuzz | Phase 1 | Tracked with the security work in [07](07-QUALITY-SECURITY-AND-COMPATIBILITY.md). |
| Accessibility (axe, contrast) | **C-019** | Needs a shell to audit. |
| Frontend reflow, focus order, coarse targets, and visual baselines (Playwright) | **C-025** | jsdom has no layout; these require a real rendering engine at the design note's named widths. |
| SPA deep-link refresh and asset-fallback exclusions | **C-027** | Requires the production web root and deployable image path. |

## Future gates

Gaps we know about that are **not** yet designed into the tables above,
recorded rather than forgotten:

- Mutation testing on `casual-task-authz` — the highest-value place for it.
- Chaos tests: database failover, object-store outage, plugin storm.
- Multi-version compatibility matrix once `/v2` exists.
- Automated GDPR-deletion verification.
- Load test with realistic concurrent-user mix, not just per-endpoint throughput.
- Automated illustration asset sub-budget and forced-colour screenshot checks.
