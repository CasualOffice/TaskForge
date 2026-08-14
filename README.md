# TaskForge

[![Status: Phase 1](https://img.shields.io/badge/status-phase%201%20usable%20core-orange.svg)](docs/06-ROADMAP-AND-DELIVERY.md)
[![Rust: MSRV 1.88](https://img.shields.io/badge/rust-MSRV%201.88-black.svg?logo=rust)](Cargo.toml)
[![License: Apache-2.0](https://img.shields.io/badge/license-Apache--2.0-blue.svg)](LICENSE)

**A work tracker whose core stays small permanently** — because extension happens
at declared seams instead of inside the core. Written in Rust, self-hostable as
one binary plus PostgreSQL, Apache-2.0.

Simple enough that a team is productive in ten minutes; rigorous enough that the
same team, five years and two million tasks later, has not outgrown it.

[**casualoffice.github.io/TaskForge**](https://casualoffice.github.io/TaskForge/)
— the short version, for sending to somebody who has not read this file. Its
source is `site/`, published by `.github/workflows/pages.yml`.

TaskForge is the work-tracking service of **Casual Office**, alongside
[OpenCalc](https://github.com/CasualOffice/opencalc) (Casual Sheets) and
[OpenDoc](https://github.com/CasualOffice/opendoc) (Casual Editor).

> **Status: Phase 1 — usable core.** The design record covers Phases 0 through 4
> (numbered documents in `docs/`, 32 accepted ADRs), and the tracker below is
> generated from it rather than written by hand.
>
> **There is a web client, and it works.** Sign in, create a project, raise a
> task with a description, assignee, priority and due date, move it on a board,
> filter a list on any column, watch a dashboard, upload an attachment. What is
> *not* finished is written down: the open questions in the tracker are open
> questions, not oversights.
>
> Gated on every pull request: the enforced dependency DAG, architecture lints,
> the database schema with row-level security proven as the non-superuser role,
> a deployable image with a verified deployment path, a deterministic
> 2,000,000-task reference corpus, an `EXPLAIN` gate over every read path, an
> axe accessibility pass, a real-browser geometry suite, and the ADR-024 bundle
> budget — currently 166 KiB of 200.
>
> Two things that gate honestly rather than flatteringly: the `EXPLAIN` gate
> runs against a reduced corpus, and **a green run does not mean the rule holds
> at reference scale** — full-text search degrades to a sequential scan under
> row-level security at 2 M tasks (**D-043**). And the Phase 0 threat-model
> review was conducted by an agent, says so, and asks to be countersigned.
>
> Live state: [docs/14-EXECUTION-TRACKER.md](docs/14-EXECUTION-TRACKER.md).

<!-- phase-1-landed:begin -->
**Phase 1 is under way.** 50 items started, 12 gated:

- **Cross-tenant property suite** (C-005) — `Gated`
- **Projects, membership, visibility** (C-006) — `Gated`
- **SSE + fan-out** (C-015) — `Gated`
- **Releases — what went out together, cut from the pipeline** (C-023) — `Gated`
- **Team scope — team as a place to stand, beside project and workspace** (C-024) — `Gated`
- **Who may raise what — `task_type_in`, decoded, enforced and offered** (C-025) — `Gated`
- **Reports — a filter plus a grouped count (ADR-027), `count` only** (C-026) — `Gated`
- **Environments as configuration — add, rename, reorder, remove** (C-028) — `Gated`
- **State-occupancy projection — `task_state_interval`, maintained and rebuildable** (C-029) — `Gated`
- **Duration measures — cycle time, lead time, throughput** (C-030) — `Gated`
- **An empty body is not a payload — the transport refuses a silent `undefined`** (C-034) — `Gated`
- **Dashboards — the four built-ins, five visualizations, no charting library** (C-035) — `Gated`
- **Identity, sessions, MFA, invitations** (C-001) — `Built`
- **Workspace, membership, teams** (C-002) — `Built`
- **Permission resolver + `/explain`** (C-003) — `Built`
- **Permission matrix + escalation suites** (C-004) — `Built`
- **Task CRUD, assignees, tags** (C-008) — `Built`
- **Comments** (C-009) — `Built`
- **Attachment pipeline** (C-010) — `Built`
- **Search projection + full-text** (C-013) — `Built`
- **Cursor pagination** (C-014) — `Built`
- **Extension point registry (core panels only)** (C-017) — `Built`
- **Bundle + a11y gates wired** (C-019) — `Built`
- **Chain of custody — team transfer, environment promotion, verification, `/me/queue`** (C-022) — `Built`
- **Stylesheet gate — one spacing scale, no duplicate rules** (C-027) — `Built`
- **Popover placement and the narrow list row — two release blockers** (C-031) — `Built`
- **The browser layer — geometry, reflow and touch targets, measured** (C-032) — `Built`
- **The list, to its own spec — status, assignee, column filters, grouping** (C-033) — `Built`
- **Projects, and the shell that stopped scrolling — create and edit a project; the rail and header stay put** (C-036) — `Built`
- **The phone, to its own spec — audit items 2, 3 and 5 closed, measured** (C-037) — `Built`
- **A workspace you can start — audit item 10; the first run is no longer a dead end** (C-038) — `Built`
- **The create-task flow, and the workspace in the header** (C-039) — `Built`
- **Attachments reach the browser — the preflight that made `docs/28` usable** (C-040) — `Built`
- **The task drawer belongs to the address — Home and Environments could not preview** (C-041) — `Built`
- **The attachment scan — `docs/28` step 4, the consumer that made uploads visible** (C-042) — `Built`
- **The `age` measure — how long open work has been waiting** (C-043) — `Built`
- **`created_vs_completed`, and the card that dragged behind the board** (C-044) — `Built`
- **Reports draws the same charts the dashboard does** (C-045) — `Built`
- **`time_in_state` — the last measure the closed set specified** (C-046) — `Built`
- **Deployment story and the public site — build-from-source compose, environment reference, README, GitHub Pages** (C-047) — `Built`
- **People in the palette — the third of "tasks, projects and people" nothing fetched** (C-048) — `Built`
- **A search result that says why it matched** (C-049) — `Built`
- **Prefix search — a word finds its task before it is finished (D-069 part one)** (C-050) — `Built`
- **Default workflow + transitions** (C-007) — `Building`
- **Activity + audit + outbox** (C-011) — `Building`
- **Filter grammar + compiler** (C-012) — `Building`
- **Notifications (in-app + email)** (C-016) — `Building`
- **Web shell, board, list, My Work, drawer, palette** (C-018) — `Building`
- **Rate limiting at the edge** (C-020) — `Building`
- **Export — CSV/JSONL of any task query, as a job** (C-021) — `Building`
<!-- phase-1-landed:end -->

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
  field has a named index, and CI asserts no sequential scan on any tenant-scale
  table for all 29 read paths, on every pull request.
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

<!-- phase-progress:begin -->
| Phase | Delivers | Gated | Progress |
| --- | --- | --- | --- |
| **0 — Foundation** | workspace, CI gates, schema + RLS, corpus, image | 13/16 (3 built) | `████████░░` 81% |
| **1 — Usable core** | auth, projects, tasks, workflow, outbox, search, **then** the web client | 12/50 (31 built, 7 building) | `██░░░░░░░░` 24% |
| 2 — Administration · 3 — Extensions · 4 — Advanced | custom roles, plugins, automation, reporting | 0/— | `░░░░░░░░░░` 0% |

*Generated from [docs/14-EXECUTION-TRACKER.md](docs/14-EXECUTION-TRACKER.md) by `scripts/phase-progress.py`, and gated in CI so it cannot go stale. **Progress counts `Gated` items only** — merged, tested, and protected by an acceptance gate ([AGENTS.md](AGENTS.md): "done means Gated"). Work that is built and tested but not yet gated is shown separately rather than counted.*
<!-- phase-progress:end -->

Full detail and exit gates: [docs/06-ROADMAP-AND-DELIVERY.md](docs/06-ROADMAP-AND-DELIVERY.md).

## Workspace

A Cargo workspace of layered crates; each layer depends only on those below it,
and an illegal dependency is a **build failure**, not a review comment. Layer
division: [docs/19-WORKSPACE-SCAFFOLD-DESIGN.md](docs/19-WORKSPACE-SCAFFOLD-DESIGN.md).

| Crate | Responsibility |
| --- | --- |
| `casual-task-model` | Bedrock: ID newtypes, the `WorkspaceScope` capability, closed enums, error codes, cursors. Depends on nothing. |
| `casual-task-authz` | **Implemented:** the resolver, the closed constraint set, the escalation ceilings, and `explain()`. Not yet: the `authz_epoch` cache |
| `casual-task-identity` | Users, workspace membership, teams, sessions, service accounts, tokens |
| `casual-task-project` | Projects, membership, environments, milestones, tags |
| `casual-task-workflow` | **Implemented:** statuses, transitions, and the fixed validation order. Not yet: status editing and migration |
| `casual-task-task` | Tasks, assignees, dependencies, subtasks, board ranks |
| `casual-task-activity` | Activity + audit record construction, the outbox event shape |
| `casual-task-attachment` | Attachment lifecycle, the scan/commit handshake |
| `casual-task-notification` | Notification construction and preference evaluation |
| `casual-task-app` | Command/query handlers; transaction boundaries; the only layer that composes domain crates |
| `casual-task-persistence` | **all** SQL in the system. Implemented: the scoped-connection seam. Not yet: the repositories |
| `casual-task-search` | **Implemented:** the filter AST and its closed field set. Not yet: the projection and the compiler |
| `casual-task-infra` | Redis, object storage, mail — each behind a trait with a local fallback |
| `casual-task-plugin-contract` | Extension points, manifest types, scopes, signing. Versioned independently. |
| `casual-task-observability` | **Implemented:** tracing, the metric registry, cardinality-bounded labels, correlation IDs. Not yet: an exporter |
| `casual-task-api` | The API binary: Axum routers, tower middleware, DTOs, OpenAPI, SSE |
| `casual-task-worker` | The worker binary: dispatch, projection, notify, webhook, scan, automation, retention |

Alongside them, `tools/` holds the machinery that makes the claims above
checkable. None of it ships in the image:

| Tool | What it is for |
| --- | --- |
| `casual-task-lint` | The architecture lints: illegal dependency, SQL outside persistence, HTTP outside the API, `OFFSET`, unbounded channels, an `AuthContext` minted off the edge |
| `casual-task-seed` | The deterministic reference corpus — 2,000,000 tasks and 39 M rows, byte-identical between runs, streamed at a ~26 MiB peak RSS |
| `casual-task-loadtest` | Measures 10 read paths against a seeded corpus and refuses to compare two runs that are not comparable |

`webapp/` is not the product frontend. It is the bundle-floor harness behind
ADR-024's budget — enough of the real dependency set to measure it honestly,
and nothing else ([webapp/BUNDLE-FLOOR.md](webapp/BUNDLE-FLOOR.md)).

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
docker compose up -d               # PostgreSQL for local development
cargo test --workspace             # test suite
cargo run -p casual-task-lint      # architecture lints (docs/15)
./scripts/verify-schema.sh         # migrations + tenant isolation + append-only
./scripts/verify-queries.sh        # no sequential scan on a tenant-scale table
./scripts/check.sh                 # the CI gate set; names anything it skipped
```

`check.sh` deliberately does **not** claim to be everything CI runs. Gates that
need Docker, `pnpm`, or `cargo-deny` are skipped loudly and listed at the end,
because a green local run with silent skips is a worse lie than an honest one.

Working with a corpus:

```sh
cargo run --release -p casual-task-seed -- --scale small --out target/corpus
#   tiny 5 MiB · small 263 MiB · reference 10.2 GiB — check disk before reference
cd target/corpus && ./load.sh "$DATABASE_URL"

cargo run --release -p casual-task-loadtest -- cases     # what is and is not measured
cargo run --release -p casual-task-loadtest -- run --help

pnpm --dir webapp install --frozen-lockfile && pnpm --dir webapp measure
```

### Deploy

```sh
cp deploy/.env.example deploy/.env && $EDITOR deploy/.env   # every CHANGE_ME
docker compose -f deploy/docker-compose.yml --env-file deploy/.env up -d
```

That pulls the published image. To build this repository instead:

```sh
docker compose -f deploy/docker-compose.yml --env-file deploy/.env up -d --build
```

Attachments upload with the stack above but stay invisible until something
scans them — that is deliberate and countersigned (D-062), because the
alternative default serves unscanned user content. Turn scanning on with the
daemon **and** the address, which are two steps so that doing one without the
other warns instead of silently not scanning:

```sh
docker compose -f deploy/docker-compose.yml --profile scanning up -d
#   then set TF_CLAMD_ADDR=clamav:3310 in deploy/.env and restart the api
```

Every variable, what it does and what happens when it is empty:
[`deploy/.env.example`](deploy/.env.example).

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
>
> **The database connection is not encrypted (D-050).** PostgreSQL must be on a
> trusted network — this host, or a private subnet you control. A managed
> PostgreSQL requiring `sslmode=require` is **not supported by this release**.
> The reason, and what holds the constraint, are in
> [docs/52](docs/52-DEPLOYMENT-GUIDE.md).

`taskforge-api` refuses to start on a bad configuration or a superuser database
role, and serves `/health/live`, `/health/ready` and `/metrics` alongside the
product API and the web client.

**It is a Phase 1 core, not a finished product.** Tasks, projects, boards,
lists, dashboards, environments, releases, attachments and the permission model
all work. They are not all `Gated`, which is the only word here that means done:
of 50 Phase 1 items, 12 carry an acceptance gate, 31 more are merged with their
tests passing, and 7 are in progress. Time tracking,
automation rules, plugins, saved reports and user-composed dashboards are
designed and not built — `docs/` says which is which, and the tracker says how
far each got, by name.

See [CONTRIBUTING.md](CONTRIBUTING.md) for the full command set and the PR
contract.

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
