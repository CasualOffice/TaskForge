# 18 — Support Matrix

Target vs implemented, per surface. **Nothing here is implemented** — the project
is in the documentation phase ([14](14-EXECUTION-TRACKER.md)). The "Phase" column
is the commitment; the "Status" column will move as code lands.

## Browsers

| Browser | Target | Notes |
| --- | --- | --- |
| Chrome / Edge | last 2 major | primary development target |
| Firefox | last 2 major | |
| Safari (macOS) | last 2 major | |
| Safari (iOS) | last 2 major | responsive; not a native app |
| Chrome (Android) | last 2 major | responsive |
| Internet Explorer | **never** | |

**No polyfills for browsers outside the matrix.** Required platform features:
ES2022, CSS custom properties, `EventSource`, `BroadcastChannel`, `IndexedDB`,
`ResizeObserver`, `Intl`. Anything below the matrix gets a clear unsupported
notice rather than a subtly broken application.

## Server platform

| | Target |
| --- | --- |
| Rust | stable, pinned; MSRV declared by ADR at Phase 0 |
| PostgreSQL | **16+** (requires `NULLS NOT DISTINCT`, modern partitioning) |
| Redis | 7+ (optional below 2 API instances) |
| Object storage | S3-compatible, or filesystem |
| OS | Linux x86-64 and aarch64 |
| Container | distroless / minimal Debian, < 100 MB |

PostgreSQL 15 is not supported: `UNIQUE NULLS NOT DISTINCT` is load-bearing for
workspace-scoped tag uniqueness ([22](22-DATABASE-SCHEMA.md)), and working around
it would mean a partial index and a second code path forever.

## Feature status

| Capability | Phase | Designed | Status |
| --- | --- | --- | --- |
| Local auth, sessions, MFA | 1 | [40](40-IDENTITY-AUTH-AND-SESSION.md) | not started |
| Workspaces, teams, projects | 1 | [03](03-DOMAIN-MODEL.md) | not started |
| Built-in roles, permission resolver | 1 | [04](04-RBAC-AND-AUTHORIZATION.md) | not started |
| `/permissions/explain` | 1 | [04](04-RBAC-AND-AUTHORIZATION.md) | not started |
| Tasks, subtasks, assignees, tags | 1 | [03](03-DOMAIN-MODEL.md) | not started |
| Comments, attachments | 1 | [28](28-ATTACHMENT-PIPELINE.md) | not started |
| Default workflow, transitions | 1 | [23](23-WORKFLOW-AND-STATE-MACHINE.md) | not started |
| Activity, audit, outbox | 1 | [25](25-EVENTS-OUTBOX-AND-AUDIT.md) | not started |
| Filters, saved views (built-in) | 1 | [27](27-FILTER-AND-SAVED-VIEW-DSL.md) | not started |
| Full-text search | 1 | [26](26-SEARCH-INDEXING-AND-QUERY.md) | not started |
| Board, list, My Work, palette | 1 | [42](42-FRONTEND-ARCHITECTURE.md) | not started |
| SSE live updates | 1 | [05](05-API-SPEC.md) | not started |
| Notifications (in-app, email) | 1 | [29](29-NOTIFICATIONS-AND-DELIVERY.md) | not started |
| Custom roles, scoped assignments | 2 | [04](04-RBAC-AND-AUTHORIZATION.md) | not started |
| Custom workflows + status migration | 2 | [23](23-WORKFLOW-AND-STATE-MACHINE.md) | not started |
| Environments, milestones, dependencies | 2 | [03](03-DOMAIN-MODEL.md) | not started |
| User saved views + sharing | 2 | [27](27-FILTER-AND-SAVED-VIEW-DSL.md) | not started |
| Audit console + export | 2 | [25](25-EVENTS-OUTBOX-AND-AUDIT.md) | not started |
| Export — CSV / JSON Lines | 2 | [38](38-REPORTING-EXPORT-AND-DASHBOARDS.md) | not started |
| SSO — OIDC | 2 | [40](40-IDENTITY-AUTH-AND-SESSION.md) | not started |
| SSO — SAML | 2 | [40](40-IDENTITY-AUTH-AND-SESSION.md) | not started |
| Declarative plugins | 3a | [34](34-PLUGIN-AND-EXTENSION-ARCHITECTURE.md) | not started |
| Remote HTTPS plugins, webhooks | 3b | [34](34-PLUGIN-AND-EXTENSION-ARCHITECTURE.md) | not started |
| Sandboxed frontend plugins | 3c | [34](34-PLUGIN-AND-EXTENSION-ARCHITECTURE.md) | not started |
| Automation rules | 4 | [36](36-AUTOMATION-RULES-DESIGN.md) | not started |
| Reports — measures, grouping, bucketing | 4 | [38](38-REPORTING-EXPORT-AND-DASHBOARDS.md) | not started |
| Dashboards — tiles, six visualizations | 4 | [38](38-REPORTING-EXPORT-AND-DASHBOARDS.md) | not started |
| Export — XLSX via OpenCalc | 4 | [38](38-REPORTING-EXPORT-AND-DASHBOARDS.md) | not started |
| Scheduled report delivery | later | [38](38-REPORTING-EXPORT-AND-DASHBOARDS.md) | **deferred** |
| User-defined SQL / BI query builder | — | [38](38-REPORTING-EXPORT-AND-DASHBOARDS.md) | **non-goal** — export to a real BI tool |
| Calendar / timeline (as plugins) | 4 | [34](34-PLUGIN-AND-EXTENSION-ARCHITECTURE.md) | not started |
| SCIM provisioning | 4 | — | not designed |
| Managed worker plugins | later | ADR-016 | **deferred** |
| External search engine | if triggered | ADR-014 | **conditional** |
| Multi-region / residency | — | — | **not designed; not promised** |
| Offline-first sync | — | — | **non-goal** |
| Email reply-to-comment | — | [29](29-NOTIFICATIONS-AND-DELIVERY.md) | **deferred, deliberately** |
| Sprints / time-boxing | — | [17](17-GLOSSARY.md) | **plugin surface, not core** |
| Time tracking | — | — | **plugin surface** |

## Accessibility

| | Target |
| --- | --- |
| Standard | **WCAG 2.2 level AA** |
| Keyboard | full operation, including drag & drop |
| Screen readers | NVDA (Windows), VoiceOver (macOS/iOS) |
| Contrast | 4.5:1 text, 3:1 UI, light and dark |
| Motion | `prefers-reduced-motion` honoured |
| Verification | axe in CI + manual keyboard pass per release |

## Localization

| | Target |
| --- | --- |
| UI strings | externalized from Phase 1; English only at launch |
| Dates, numbers | `Intl`, user locale |
| **Timezones** | per user, applied to all relative filters ([27](27-FILTER-AND-SAVED-VIEW-DSL.md)) |
| Text direction | LTR at launch; RTL not committed |
| Search language | English stemming at launch; per-workspace config designed but not shipped |

Timezone correctness is Phase 1, not a later polish item: `due before @today`
must mean the same thing in Auckland and Los Angeles, and retrofitting that is
far harder than building it in.

## API & contract stability

| Surface | Guarantee |
| --- | --- |
| REST `/api/v1` | stable; additive only; breaking ⇒ `/v2` + overlap window |
| Event payloads | per-type `schema_version`; both delivered during deprecation |
| Plugin contract | semver, independent of app version (ADR-015) |
| Error codes | append-only, never reused ([20](20-ERROR-CODE-REGISTRY.md)) |
| Database schema | forward-only; rollback-safe one version |
| Cursors | opaque; shape may change without notice |

## How to read this document

- **not started** — designed, not built.
- **not designed** — acknowledged, no design note yet. Do not promise it.
- **deferred** — deliberately postponed, with a recorded reason.
- **non-goal** — will not be built ([01](01-ORD.md)).
- **conditional** — built only if a named tripwire fires.

The distinction between *not designed* and *deferred* matters: the first is a gap,
the second is a decision.
