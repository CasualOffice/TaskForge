# Changelog

All notable changes to TaskForge are recorded here.

Format: [Keep a Changelog](https://keepachangelog.com/en/1.1.0/).
Versioning: [Semantic Versioning](https://semver.org/) against the **public API**
surface, not the internal crates.

Four things version independently — the REST API, the database schema, the event
schema, and the plugin contract ([docs/02](docs/02-ARCHITECTURE.md)). A change to
one does not imply a change to the others, and each is called out separately below
when it moves.

## [Unreleased]

### Added — Phase 0 foundation

No product functionality. Everything here exists to make later phases
verifiable, and each item is tracked in
[docs/14](docs/14-EXECUTION-TRACKER.md).

- **Database schema (F-015)** — 12 migrations, the non-superuser
  `taskforge_app` role, and a verification gate that applies every migration to
  a clean PostgreSQL 16 and proves tenant isolation and append-only history *as
  that role*, because neither guarantee holds for a superuser.
- **Deployable image (F-016)** — multi-stage build, non-root uid 65532, under
  the 100 MB budget, with an end-to-end gate that brings the deployment compose
  up and asserts it is actually isolated.
- **Reference corpus (F-006)** — `tools/casual-task-seed` generates the docs/30
  workspace deterministically: 2,000,000 tasks and 38,981,941 rows as
  PostgreSQL `COPY` files in 18.2 s at a 26 MiB peak RSS. Byte-identity across
  runs, one-level threading, and manifest accuracy are gated by tests.
- **Latency harness (F-007)** — `tools/casual-task-loadtest` measures 10 read
  paths and compares against a committed baseline, refusing to compare at all
  when the environment, corpus size, or a case's returned-row count has moved.
  Built, not gated: there is no reference machine to measure a baseline on.
- **`EXPLAIN` no-seq-scan gate (F-008)** — all 20 read paths planned as
  `taskforge_app` with the row-level-security predicate in the plan; 20
  index-served, 0 sequential scans. Runs per pull request.
- **Bundle budget (F-012)** — the ADR-024 shell budget measured rather than
  assumed: a 113.2 KiB gzip dependency floor against 200 KiB, with the unit,
  the definition of "initial", and the bundler's 4.4% influence all pinned down.
  Runs per pull request.
- **Observability skeleton (F-009)** — metrics, label-cardinality types,
  redaction, correlation, and a JSON subscriber. Building; see docs/14 for the
  two defects blocking it.

### Added — design record

The complete design record for Phases 0–4: 37 numbered documents in `docs/`, and
26 Accepted ADRs in [docs/08-ADR-REGISTER.md](docs/08-ADR-REGISTER.md).

Decisions of record, previously undecided or unwritten:

- **ADR-001** — Rust/Axum/SQLx; product name TaskForge, crates `casual-task-*`.
  Supersedes the archived Java/Spring drafts.
- **ADR-004** — Additive-union RBAC with no deny rules, and an explainable
  decision function.
- **ADR-006** — Transactional outbox from the first mutation.
- **ADR-009** — Closed, typed extension point registry; core features render
  through it.
- **ADR-010** — Multiple assignees with an optional primary; single-select
  environment. (Two questions the old drafts left open.)
- **ADR-011** — Closed filterable/sortable field set, one named index each.
- **ADR-014** — PostgreSQL-native search, with a measured tripwire for revisiting.
- **ADR-024** — Client bundle budget measured before it is promised.
- **ADR-025** — Audit retention 400 days; IP/user-agent captured, with a stated
  reason.

### Added — repository

- Root governance: `README.md`, `AGENTS.md`, `CLAUDE.md`, `SKILLS.md`,
  `CONTRIBUTING.md`, `SECURITY.md`, `GOVERNANCE.md`, `CODE_OF_CONDUCT.md`,
  `LICENSE` (Apache-2.0).
- Cargo workspace scaffold: 17 crates plus tooling, with the dependency DAG from
  [docs/19](docs/19-WORKSPACE-SCAFFOLD-DESIGN.md) declared so an illegal
  dependency is a build failure.

### Changed

- **Documentation reorganized** into the `NN-TITLE.md` convention shared with
  OpenDoc and OpenCalc, with a single owner per fact.

### Removed

- Two byte-identical duplicate drafts.

### Deprecated

- The pre-reorganization drafts are retained in [`docs/_archive/`](docs/_archive/README.md)
  for provenance only. They describe a **Java/Spring** backend that had already
  been superseded and are **not authoritative**.

### Known open decisions

Tracked in [docs/14-EXECUTION-TRACKER.md](docs/14-EXECUTION-TRACKER.md), not
resolved:

- **D-032** — auth protocol specifics (session vs bearer details, refresh, OIDC
  claim mapping). To be Accepted at Phase 0.
- **D-033** — custom-field value storage. Before Phase 3.
- **D-034** — multi-region / data residency. **Not designed; must not be promised
  to a customer** until it is.

---

## Release notes format

Each future release will record, under its version:

- **Added / Changed / Deprecated / Removed / Fixed / Security** — the usual
  sections.
- **API** — `/api/v1` changes, marked additive or breaking.
- **Schema** — migrations included, with expand/contract phase.
- **Events** — any `schema_version` bump, and the deprecation window.
- **Plugin contract** — semver movement and compatibility impact.
- **Gates** — new acceptance gates, and any gate whose baseline moved (with the
  reason the regression was accepted).

No release is published without a verified restore drill and a timed migration
rehearsal ([docs/15](docs/15-CI-AND-RELEASE-GATES.md)).
