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
| `cache-key-scoped` | every cache key carries workspace, principal id and type, optional project, and epoch | enforced by the only public key type; isolation and epoch-miss tests block CI |
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
| Numbers the README states about CI are derived, not typed | ✅ |
| Tracker counts stated anywhere match [14](14-EXECUTION-TRACKER.md) | ✅ |

The filter-field gate is the one that keeps the index contract honest over time:
you cannot add a way to query without adding the index that serves it, in the
same PR.

The derived-numbers gate is two checks in the `documentation` job, and both
exist because the same thing happened twice: a figure stated in the README has
no reason to change when the thing it describes does. `phase-progress.py
--check` holds the phase table and the landed list against
[14](14-EXECUTION-TRACKER.md); `check-read-path-count.py` holds the read-path
count against `tests/explain/queries/`, which is what `verify-queries.sh`
actually globs. That count had drifted to 23 while the corpus reached 29 — and
it is the one figure that tells a reader how much of the product the
no-sequential-scan guarantee covers, so a stale one misstates the guarantee.
The same check rejects duplicate `NN-` prefixes in the corpus, because three
were used twice and a numbering scheme that does not number is read as an index
anyway.

`check-status-counts.py` is the third of the family and the widest: five files —
the README, both agent contracts, and both halves of the public site — state how
many tracker rows are `Gated`, `Built` and `Building`, because that is the honest
answer to "how finished is this?" and it belongs where somebody is reading rather
than one link away. Five copies is five chances to be wrong, and this repository
has been wrong in both directions: the published site said "built and gated" over
rows marked `Building`, and `AGENTS.md` said "no product functionality exists yet"
long after there was a product. Adding one tracker row then moves every count at
once. It asserts the digits rather than generating the sentences, because the
wording differs per audience and generating it would flatten a page written for a
person into a page written for a script — and it fails loudly if a sentence is
reworded out from under it, since a gate that quietly stopped looking is worse
than no gate.

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
| Latency (subset + full) | **F-007** | The harness and the comparison gate are built and tested. There is no baseline to compare against: `benchmarks/reference-8vcpu-32gb.reference.json` is a placeholder that no run can pass, because the docs/30 reference machine does not exist yet. |
| ~~Frontend lint (`eslint`, `stylelint`)~~ | — | **Built (C-019).** `webapp/eslint.config.js` and the stylelint rules run as `pnpm lint` and `pnpm lint:css` in the `frontend-a11y` job. |
| ~~Frontend tests (Vitest), E2E (Playwright)~~ | — | **Built (C-018, C-019).** `pnpm test` (Vitest) and `pnpm e2e` (Playwright, desktop and phone projects) run in the `frontend-a11y` job. `webapp/` stopped being the bundle-floor harness at C-018. |
| ~~Integration (testcontainers)~~ | — | **Built (F-005).** `crates/casual-task-persistence/tests/schema_harness.rs` starts PostgreSQL 16, applies every migration, and reaches the invariants from Rust. Run by the `schema` job. The tests are `#[ignore]` so `cargo test` stays runnable without a Docker daemon; CI runs them explicitly, because otherwise nothing would. |
| Query count (no N+1) | **C-012** | Needs a query layer to count. |
| ~~Permission matrix, escalation, cross-tenant~~ | — | **Built (C-004) / Gated (C-005).** `authz.rs`, `permissions.rs`, `roles.rs`, `tenant_isolation.rs`, and the route-derived `route_isolation.rs` sweep run through `cargo test --workspace -- --ignored` in the blocking `schema` job. D-056 still blocks the final built-in-role matrix. |
| OpenAPI diff, event schema diff, plugin contract diff, error registry | Phase 1 | Nothing to diff until `/v1` and the registry exist. |
| Secret scan, SAST, container scan, enumeration, injection, fuzz | Phase 1 | Tracked with the security work in [07](07-QUALITY-SECURITY-AND-COMPATIBILITY.md). |
| ~~Accessibility (axe, contrast)~~ | — | **Built (C-019).** axe runs over rendered output in `pnpm test`; the `frontend-a11y` job is named for it. |

## Future gates

Gaps we know about that are **not** yet designed into the tables above,
recorded rather than forgotten:

- Mutation testing on `casual-task-authz` — the highest-value place for it.
- Chaos tests: database failover, object-store outage, plugin storm.
- Multi-version compatibility matrix once `/v2` exists.
- Automated GDPR-deletion verification.
- Load test with realistic concurrent-user mix, not just per-endpoint throughput.
