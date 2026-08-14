# 15 — CI & Release Gates

The PR contract. The tables name what is required to block merge. The
**Pending gates** table records any requirement whose harness is not yet in CI;
those rows are gaps, not green checks. Disabling an implemented gate requires
an ADR.

## Hosted-action runtime

GitHub-hosted JavaScript actions are executable CI dependencies. A per-PR
workflow must use action majors whose embedded Node runtime is supported by the
hosted runner; a deprecation annotation is a maintenance failure even while the
job still exits successfully. Runtime compatibility is repaired by upgrading
the producing action and rerunning the complete gate set. The workflow must not
set GitHub's force-runtime compatibility variable, because that executes an old
bundle under a runtime its publisher did not declare.

The acceptance evidence is the next `main` run: all required jobs pass and its
annotations contain no embedded-Node deprecation. The cost is taking action
major upgrades before a failing deadline, so each refresh is isolated from
product behavior and reviewed through the same blocking gates it changes.

## Per-PR gates

### Build & style

| Gate | Command | Required |
| --- | --- | --- |
| Format | `cargo fmt --check` | ✅ |
| Lint | `cargo clippy --all-targets -- -D warnings` | ✅ |
| Build | `cargo build --workspace --all-features` | ✅ |
| Docs build | `cargo doc --no-deps` | ✅ |
| Dependencies | `cargo deny check` (licenses, advisories, bans, sources) | ✅ |
| Frontend lint/types | `eslint`, `tsc --noEmit` | ✅ |
| Module responsibility bound | `scripts/check-module-size.py` rejects Rust, TypeScript, TSX or CSS modules over 500 lines | ✅ |

### Correctness

| Gate | What | Required |
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

| Gate | What | Required |
| --- | --- | --- |
| OpenAPI diff | vs committed snapshot; breaking change requires a version bump | ✅ |
| Event schema diff | payload changes require a `schema_version` bump | ✅ |
| Plugin contract diff | semver-checked | ✅ |
| Error registry | every emitted code exists in [20](20-ERROR-CODE-REGISTRY.md) | ✅ |

### Performance

| Gate | What | Required |
| --- | --- | --- |
| **`EXPLAIN` no-seq-scan** | every endpoint × sortable field, reference corpus | ✅ |
| Query count | no N+1; one authorization resolution per list | ✅ |
| **Bundle size** | shell ≤ ADR-024 budget | ✅ |
| Latency (subset) | reduced corpus, >10% regression vs baseline | ✅ |
| Latency (full) | full reference corpus | nightly |

### Schema & deployment

| Gate | What | Required |
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

| Gate | What | Required |
| --- | --- | --- |
| Secret scan | no credentials in the diff (incl. `tf_pat_`/`tf_sat_` prefixes) | ✅ |
| SAST | `cargo deny check advisories` + repository security rules | ✅ |
| Container scan | base image CVEs | ✅ |
| Enumeration test | login/reset/invite responses indistinguishable | ✅ |
| Injection property test | filter compiler emits no user-derived SQL strings | ✅ |
| Fuzz (smoke) | filter grammar + plugin contract identifiers, short budget | ✅ |
| Fuzz (deep) | extended budget | nightly |

### Accessibility

| Gate | What | Required |
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
| `no-io-in-transaction` | no storage, scanner, mail or broadcast call between transaction begin and commit | enforced over API and worker sources; the attachment commit/scan split is the regression case |
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

- **Static release readiness**: D-048 is Accepted, every Dockerfile base is
  digest-pinned, and an existing-volume upgrade runner exists. The release
  workflow blocks publication while any of these are false.

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

The tables above are the **required contract**, not a claim that every harness
exists. Missing harnesses are listed here with the tracker item that lands each
one. `.github/workflows/ci.yml` points at this section, and the rule is: a gate
is either a blocking workflow step or a row in this table. Never neither.

| Gate (from the tables above) | Lands with | Why not yet |
| --- | --- | --- |
| Latency (subset + full) | **F-007** | The harness and the comparison gate are built and tested. There is no baseline to compare against: `benchmarks/reference-8vcpu-32gb.reference.json` is a placeholder that no run can pass, because the docs/30 reference machine does not exist yet. |
| ~~Frontend lint (`eslint`, `stylelint`)~~ | — | **Built (C-019).** `webapp/eslint.config.js` and the stylelint rules run as `pnpm lint` and `pnpm lint:css` in the `frontend-a11y` job. |
| ~~Frontend tests (Vitest), E2E (Playwright)~~ | — | **Built (C-018, C-019).** `pnpm test` (Vitest) and `pnpm e2e` (Playwright, desktop and phone projects) run in the `frontend-a11y` job. `webapp/` stopped being the bundle-floor harness at C-018. |
| ~~Integration (testcontainers)~~ | — | **Built (F-005).** `crates/casual-task-persistence/tests/schema_harness.rs` starts PostgreSQL 16, applies every migration, and reaches the invariants from Rust. Run by the `schema` job. The tests are `#[ignore]` so `cargo test` stays runnable without a Docker daemon; CI runs them explicitly, because otherwise nothing would. |
| ~~Query count (no N+1)~~ | — | **Built (C-012).** The 100-task list integration test asserts one authorization resolution for the whole page through the exported resolution metric. |
| ~~Permission matrix, escalation, cross-tenant~~ | — | **Built (C-004) / Gated (C-005).** `authz.rs`, `permissions.rs`, `roles.rs`, `tenant_isolation.rs`, and the route-derived `route_isolation.rs` sweep run through `cargo test --workspace -- --ignored` in the blocking `schema` job. D-056 still blocks the final built-in-role matrix. |
| OpenAPI diff | Phase 1 contract artifact | `/api/v1` exists, but there is no canonical committed OpenAPI snapshot. Generating and reviewing that public artifact is design work; until it lands, compatibility changes are not machine-checked. |
| Event schema diff | **D-053** | Events carry `schema_version`, but D-053's event registry is open and there is no canonical payload-schema snapshot to compare. |
| Plugin contract diff | Phase 1 contract artifact | Contract identifiers and versions exist, but CI has no committed compatibility baseline. Fuzzing checks parser safety, not semantic-version compatibility. |
| ~~Error registry~~ | — | **Built.** `check-error-registry.py` extracts compile-time `Code::new`/`ErrorCode::new` declarations and rejects any code absent from `docs/20`; the documentation job blocks on it. |
| ~~Security static rules and container scan~~ | — | **Built.** Dependency advisories, secret scanning and TaskForge-specific source rules block CI; the image job scans the built runtime with Trivy and fails on fixed high/critical findings. |
| ~~Enumeration and injection~~ | — | **Built.** Login/reset/invite indistinguishability is exercised against PostgreSQL; filter compilation tests prove user values remain bound parameters. Both run in blocking test jobs. |
| ~~Fuzz smoke~~ | — | **Built.** The `fuzz-smoke` job gives the recursive filter JSON surface and plugin-contract identifiers bounded libFuzzer budgets on every PR. Deep campaigns remain nightly. |
| ~~Accessibility (axe)~~ | — | **Built (C-019).** axe runs over rendered output in `pnpm test`; the `frontend-a11y` job is named for it. |
| Contrast automation | **C-019** | jsdom has no layout and cannot evaluate color contrast. CI checks token use; the actual light/dark contrast pass is still a release check. |
| Migration rehearsal from a prior production version, with timing | Release engineering | CI applies every migration to a clean PostgreSQL 16. It does not yet restore a production-shaped prior version and time the upgrade. |

## Future gates

Gaps we know about that are **not** yet designed into the tables above,
recorded rather than forgotten:

- Mutation testing on `casual-task-authz` — the highest-value place for it.
- Chaos tests: database failover, object-store outage, plugin storm.
- Multi-version compatibility matrix once `/v2` exists.
- Automated GDPR-deletion verification.
- Load test with realistic concurrent-user mix, not just per-endpoint throughput.
