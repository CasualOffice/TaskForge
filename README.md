# TaskForge

[![Status: Phase 0](https://img.shields.io/badge/status-phase%200%20foundation-orange.svg)](docs/06-ROADMAP-AND-DELIVERY.md)
[![Rust: MSRV 1.90](https://img.shields.io/badge/rust-MSRV%201.90-black.svg?logo=rust)](rust-toolchain.toml)
[![License: Apache-2.0](https://img.shields.io/badge/license-Apache--2.0-blue.svg)](LICENSE)

**A work tracker whose core stays small permanently** — because extension happens
at declared seams instead of inside the core. Written in Rust, self-hostable as
one binary plus PostgreSQL, Apache-2.0.

Simple enough that a team is productive in ten minutes; rigorous enough that the
same team, five years and two million tasks later, has not outgrown it.

TaskForge is the work-tracking service of **Casual Office**, alongside
[OpenCalc](https://github.com/CasualOffice/opencalc) (Casual Sheets) and
[OpenDoc](https://github.com/CasualOffice/opendoc) (Casual Editor).

> **Status: Phase 0 — foundation.** The design record is complete (numbered
> documents in `docs/`, 30 accepted ADRs, covering Phases 0 through 4). Landed
> and gated: the Cargo workspace and its enforced dependency DAG, architecture
> lints, the full database schema with row-level security proven against a real
> PostgreSQL 16, and a deployable container image with a verified deployment
> path.
>
> **No product functionality exists yet** — the binaries are scaffolds, by
> design. Phase 0 builds none; it exists to make every later phase verifiable.
> Live state: [docs/14-EXECUTION-TRACKER.md](docs/14-EXECUTION-TRACKER.md).

## The problem

Work tracking forces a bad trade:

- **Heavyweight trackers** can model any process, but every team pays for
  configuration surface it will never use.
- **Lightweight trackers** feel good until the first real requirement lands — a
  per-project role, a QA gate, an environment, an auditable history — and there
  is no seam to add it.
- **Self-hosted open source** gets you control, but extension usually means
  forking the core, and the fork can never be upgraded.

TaskForge is built the other way around: a deliberately small core, with a
**closed, typed extension point registry** that adding a plugin never has to
modify.

## What makes it different

- **Permissions you can explain.** Effective access is the additive union of
  grants — no deny rules, no precedence puzzles. `POST /permissions/explain`
  answers "why can't I close this?" with the actual contributing grants
  ([docs/04](docs/04-RBAC-AND-AUTHORIZATION.md)).
- **Status is yours; state is ours.** Teams rename and rewire statuses freely
  above five permanent semantic states, so reports, automations, and plugins keep
  working forever ([docs/23](docs/23-WORKFLOW-AND-STATE-MACHINE.md)).
- **Nothing scans.** The filterable and sortable field set is **closed**, each
  field has a named index, and CI asserts no sequential scan on a 2M-task corpus.
  A filter on an unlisted field is a `400`, not a slow query
  ([docs/26](docs/26-SEARCH-INDEXING-AND-QUERY.md)).
- **Open for extension, closed for modification.** Adding a plugin never changes
  core code; adding a new *kind* of extension point does, and requires an ADR.
  Core panels render through the same registry plugins use
  ([docs/34](docs/34-PLUGIN-AND-EXTENSION-ARCHITECTURE.md)).
- **Complete, immutable history.** The domain change, its activity record, its
  audit record, and its outbox event commit in one transaction — there is no
  window in which a change exists without its history
  ([docs/25](docs/25-EVENTS-OUTBOX-AND-AUDIT.md)).
- **Tenant isolation by construction.** A `WorkspaceScope` capability type that
  only authenticated middleware can mint, with PostgreSQL row-level security as
  an independent backstop. Two mechanisms must both fail to leak
  ([docs/32](docs/32-TENANCY-AND-ISOLATION.md)).
- **One binary and PostgreSQL is a real deployment.** Redis, object storage, and
  a separate worker process are all optional — a design constraint that shaped
  the architecture, not a convenience ([docs/48](docs/48-DEPLOYMENT-PROFILES.md)).

## The simplicity contract

The hardest requirement in the product, and the one most likely to be violated
quietly:

> **Adding a capability must not add a concept.**

A feature earns its place by fitting an existing noun — a task type, a status, a
permission, an extension point, a filter field. A new top-level noun requires an
ADR arguing it is unavoidable. This is why there are eleven user-facing nouns, and
why there are no sprints and no epics ([docs/17](docs/17-GLOSSARY.md)).

## Scope, in a planned order

| Phase | Delivers | Status |
| --- | --- | --- |
| **0 — Foundation** | workspace + enforced layer division, CI gates, architecture lints, **schema + RLS + deployment image**, reference corpus, load-test harness, observability skeleton | 🟡 in progress |
| 1 — Usable core | auth, workspaces, projects, tasks, comments, attachments, default workflow, **the full permission resolver**, activity/audit/outbox, filters, search, board/list/My Work, SSE, notifications | ⬜ |
| 2 — Administration | custom roles, permission simulator, custom workflows + status migration, environments, milestones, dependencies, audit console, SSO | ⬜ |
| 3 — Extension platform | declarative plugins → remote HTTPS → sandboxed frontend, integration SDK | ⬜ |
| 4 — Advanced productivity | automation engine, reporting, calendar/timeline **as plugins**, SCIM | ⬜ |

Full detail and exit gates: [docs/06-ROADMAP-AND-DELIVERY.md](docs/06-ROADMAP-AND-DELIVERY.md).

## Workspace

A Cargo workspace of layered crates; each layer depends only on those below it,
and an illegal dependency is a **build failure**, not a review comment. Layer
division: [docs/19-WORKSPACE-SCAFFOLD-DESIGN.md](docs/19-WORKSPACE-SCAFFOLD-DESIGN.md).

| Crate | Responsibility |
| --- | --- |
| `casual-task-model` | Bedrock: ID newtypes, the `WorkspaceScope` capability, closed enums, error codes, cursors. Depends on nothing. |
| `casual-task-authz` | The permission resolver, constraints, ceilings, the `authz_epoch` cache, and `explain()` |
| `casual-task-identity` | Users, workspace membership, teams, sessions, service accounts, tokens |
| `casual-task-project` | Projects, membership, environments, milestones, tags |
| `casual-task-workflow` | Workflows, statuses, transitions, state mapping, transition validation |
| `casual-task-task` | Tasks, assignees, dependencies, subtasks, board ranks |
| `casual-task-activity` | Activity + audit record construction, the outbox event shape |
| `casual-task-attachment` | Attachment lifecycle, the scan/commit handshake |
| `casual-task-notification` | Notification construction and preference evaluation |
| `casual-task-app` | Command/query handlers; transaction boundaries; the only layer that composes domain crates |
| `casual-task-persistence` | SQLx repositories — **all** SQL in the system |
| `casual-task-search` | Search projection and query construction |
| `casual-task-infra` | Redis, object storage, mail — each behind a trait with a local fallback |
| `casual-task-plugin-contract` | Extension points, manifest types, scopes, signing. Versioned independently. |
| `casual-task-observability` | Tracing, metrics, correlation IDs |
| `casual-task-api` | The API binary: Axum routers, tower middleware, DTOs, OpenAPI, SSE |
| `casual-task-worker` | The worker binary: dispatch, projection, notify, webhook, scan, automation, retention |

## Prior art we study

Openly, and on the record ([docs/12](docs/12-COMPETITIVE-ANALYSIS.md)):

- **Configurability and its cost** — Jira (the status-category idea, taken; scheme
  indirection, rejected), Azure DevOps, ServiceNow.
- **The feel bar** — Linear (command palette, optimistic UI, sub-100 ms
  interactions), Trello, Height.
- **Extension models** — Atlassian Forge/Connect and GitHub Apps (scopes, consent,
  independently versioned platform contract — taken substantially); **Redmine**
  (in-process monkey-patching — the worked negative example our registry is
  designed against).
- **The category** — OrangeScrum, studied as published behaviour only. TaskForge
  is a **clean-room** implementation: no source, schema, template, or asset is
  copied ([docs/09](docs/09-REPOSITORY-AND-CONTRIBUTION.md)).

## Getting started

### Develop

```sh
docker compose up -d              # PostgreSQL for local development
cargo test --workspace            # test suite
cargo run -p casual-task-lint     # architecture lints (docs/15)
./scripts/verify-schema.sh        # apply migrations + assert the invariants
./scripts/check.sh                # everything CI runs
```

### Deploy

```sh
cp deploy/.env.example deploy/.env && $EDITOR deploy/.env
docker compose -f deploy/docker-compose.yml --env-file deploy/.env up -d
```

One binary plus PostgreSQL — no Redis, no object storage, no message broker.
Keeping that profile genuinely supported is a **constraint on the architecture**,
not a convenience. Full walkthrough: [docs/52](docs/52-DEPLOYMENT-GUIDE.md).

| | |
| --- | --- |
| Image | `gcr.io/distroless/cc-debian12:nonroot`, ~49 MB, runs as uid 65532 |
| Contains | `taskforge-api`, `taskforge-worker`, and the migrations |
| Verified by | `./scripts/verify-deployment.sh`, gated in CI |

> **The application does not connect as the database owner.** A superuser
> bypasses row-level security unconditionally, which would make tenant isolation
> and audit immutability both silently inert. `deploy/` sets this up correctly
> and CI asserts it — see [docs/52](docs/52-DEPLOYMENT-GUIDE.md).

**There is no runnable application yet** — the binaries are Phase 0 scaffolds.
The image, the schema, and the deployment path are real and gated. See
[CONTRIBUTING.md](CONTRIBUTING.md) for the full command set and the PR contract.

## Documentation

`docs/` is the **source of truth**. Code follows docs.

Start at [docs/00-README.md](docs/00-README.md). The four load-bearing documents,
which should be read before writing any code:

- [04 — RBAC & Authorization](docs/04-RBAC-AND-AUTHORIZATION.md)
- [26 — Search, Indexing & Query](docs/26-SEARCH-INDEXING-AND-QUERY.md)
- [34 — Plugin & Extension Architecture](docs/34-PLUGIN-AND-EXTENSION-ARCHITECTURE.md)
- [25 — Events, Outbox & Audit](docs/25-EVENTS-OUTBOX-AND-AUDIT.md)

Every decision is in one table: [docs/08-ADR-REGISTER.md](docs/08-ADR-REGISTER.md).

## Contributing

See [CONTRIBUTING.md](CONTRIBUTING.md), [AGENTS.md](AGENTS.md) (the contract for
coding agents), and [SECURITY.md](SECURITY.md).

Contributions are under Apache-2.0 by the DCO. There is no CLA.

## License

Apache-2.0 — see [LICENSE](LICENSE).
