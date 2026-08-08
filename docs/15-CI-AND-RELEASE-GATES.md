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

| Lint | Rule |
| --- | --- |
| `no-sql-outside-persistence` | `sqlx::query*` only in `casual-task-persistence` |
| `no-http-outside-api` | HTTP types only in `casual-task-api` |
| `no-cross-domain-dep` | no domain crate depends on another domain crate |
| `scope-required` | every repository method takes a `WorkspaceScope` |
| `no-offset` | `OFFSET` is banned in application SQL |
| `bounded-channels` | no unbounded channel constructor |
| `no-io-in-transaction` | no HTTP/object-store client reachable from a transaction scope |
| `cache-key-scoped` | every cache key constructor takes a `WorkspaceScope` |
| `event-in-transaction` | handlers return events; they cannot publish directly |

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

## Future gates

Gaps we know about, recorded rather than forgotten:

- Mutation testing on `casual-task-authz` — the highest-value place for it.
- Chaos tests: database failover, object-store outage, plugin storm.
- Multi-version compatibility matrix once `/v2` exists.
- Automated GDPR-deletion verification.
- Load test with realistic concurrent-user mix, not just per-endpoint throughput.
