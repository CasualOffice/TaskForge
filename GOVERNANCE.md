# Governance

TaskForge is maintained under the CasualOffice GitHub organization, alongside its
sibling engines OpenDoc (`.docx`) and OpenCalc (`.xlsx`).

## Roles

**Maintainers** own repository administration, releases, security response, and
final decisions on compatibility and the ADR register.

**Subsystem owners** review changes in their documented area and maintain its
design notes, tests, and tracker state. The subsystems are:

| Subsystem | Primary docs |
| --- | --- |
| Authorization & tenancy | [04](docs/04-RBAC-AND-AUTHORIZATION.md), [32](docs/32-TENANCY-AND-ISOLATION.md) |
| Domain & workflow | [03](docs/03-DOMAIN-MODEL.md), [23](docs/23-WORKFLOW-AND-STATE-MACHINE.md) |
| Data, search & indexing | [22](docs/22-DATABASE-SCHEMA.md), [26](docs/26-SEARCH-INDEXING-AND-QUERY.md), [27](docs/27-FILTER-AND-SAVED-VIEW-DSL.md) |
| Events, audit & workers | [25](docs/25-EVENTS-OUTBOX-AND-AUDIT.md), [36](docs/36-AUTOMATION-RULES-DESIGN.md) |
| API & contracts | [05](docs/05-API-SPEC.md), [20](docs/20-ERROR-CODE-REGISTRY.md), [21](docs/21-API-LIMITS-AND-QUOTAS.md) |
| Extension platform | [34](docs/34-PLUGIN-AND-EXTENSION-ARCHITECTURE.md) |
| Identity & auth | [40](docs/40-IDENTITY-AUTH-AND-SESSION.md) |
| Frontend | [42](docs/42-FRONTEND-ARCHITECTURE.md) |
| Operations | [46](docs/46-OBSERVABILITY-AND-OPERATIONS.md), [48](docs/48-DEPLOYMENT-PROFILES.md) |

**Contributors** may propose designs and changes through the documented process
([CONTRIBUTING.md](CONTRIBUTING.md)).

Named maintainers and subsystem owners will be recorded before the first public
preview. Until then, repository write access is the authoritative maintainer
signal.

## How decisions are made

**Design decisions are made in writing, before code**
([docs/11-DESIGN-FIRST-PROCESS.md](docs/11-DESIGN-FIRST-PROCESS.md)).

1. A design note is proposed as a numbered document in `docs/`.
2. If it trips an **ADR trigger**, an ADR is written and must be **Accepted**
   before implementation.
3. Discussion happens on the PR that introduces the note.
4. A maintainer marks it final.

**ADRs are append-only.** A decision is superseded by a new ADR, never edited
away. The register ([docs/08-ADR-REGISTER.md](docs/08-ADR-REGISTER.md)) is the
complete decision history, and it is the first thing a new contributor should
read.

### Reversing a decision

Any Accepted ADR can be reversed — by a **superseding ADR** that states what
changed, what evidence prompted it, and what it costs. What is not acceptable is
reversing a decision implicitly in an implementation, or under deadline pressure
without the record.

The additive-permission model ([ADR-004](docs/08-ADR-REGISTER.md)) is the
decision most likely to face this pressure. Its cost is documented precisely so
that any future reversal argues against a stated position rather than a
half-remembered one.

## Scope disputes

The most common disagreement in a product like this is whether something belongs
in the core. The tie-breaker is the **simplicity contract**
([docs/01-ORD.md](docs/01-ORD.md)):

> Adding a capability must not add a concept.

If a proposed feature needs a new top-level noun in the user's vocabulary, it
needs an ADR arguing the noun is unavoidable. If it fits an existing noun — a task
type, a status, a permission, an extension point, a filter field — it is an
ordinary change. If it needs neither, it belongs in a plugin
([docs/34](docs/34-PLUGIN-AND-EXTENSION-ARCHITECTURE.md)).

The burden of proof is on the addition.

## Releases

- `main` is always releasable; all gates must pass
  ([docs/15-CI-AND-RELEASE-GATES.md](docs/15-CI-AND-RELEASE-GATES.md)).
- Releases follow semantic versioning against the **public API** surface, not the
  internal crates.
- Four things version independently: the REST API, the database schema, the event
  schema, and the plugin contract
  ([docs/02-ARCHITECTURE.md](docs/02-ARCHITECTURE.md)).
- Breaking changes are called out explicitly in `CHANGELOG.md`.
- Security fixes ship out of band.

## Compatibility commitments

Maintainers are accountable for the guarantees in
[docs/07-QUALITY-SECURITY-AND-COMPATIBILITY.md](docs/07-QUALITY-SECURITY-AND-COMPATIBILITY.md):
additive-only API changes within a major version, append-only error codes, an
independently versioned plugin contract, and forward-only rollback-safe
migrations.

**A commitment not yet made is not a commitment.** Anything marked *not designed*
in [docs/18-SUPPORT-MATRIX.md](docs/18-SUPPORT-MATRIX.md) — notably multi-region
data residency — must not be promised to a user before its ADR exists.

## Relationship to the suite

TaskForge shares the design system, the Rust toolchain and CI shape, the
numbered-docs process, and Apache-2.0 with OpenDoc and OpenCalc.

It does **not** share their runtime. They are embeddable single-process engines;
TaskForge is a multi-tenant service. Process is shared; architecture is not.

## Code of conduct

[CODE_OF_CONDUCT.md](CODE_OF_CONDUCT.md) applies to all project spaces.
Maintainers are responsible for enforcement.
