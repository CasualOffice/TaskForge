# 34 — Plugin & Extension Architecture

How TaskForge grows horizontally without the core growing. This is the mechanism
behind the product principle "complexity at the edges" and the reason the core can
credibly promise to stay small ([01](01-ORD.md)).

## The Open/Closed contract, stated precisely

"Open for extension, closed for modification" is easy to say and easy to violate.
The precise claim TaskForge makes:

> **Adding a plugin never changes core code, core schema, or the core bundle.**
> **Adding a new *kind* of extension point does change core code — and requires an ADR.**

Two different things, deliberately separated:

| | Frequency | Cost | Gate |
| --- | --- | --- | --- |
| A new **plugin** | constant | zero core change | admin consent |
| A new **extension point** | rare | core change + version bump | ADR |

The extension point registry is therefore the real design artifact. Get the seams
right and the core stops growing; get them wrong and every integration becomes a
core patch. That is the failure mode this doc exists to prevent.

**The self-check that keeps it honest:** core features render through the same
registry plugins use. The task detail drawer's own panels (Description, Subtasks,
Comments, History, Files) are registered extensions with a `core:` namespace, not
hardcoded JSX. If the registry cannot express a core panel, it cannot express a
plugin's either — and we find that out in Phase 1, not Phase 3.

## The extension point registry (v1 — closed set)

Each point has a stable name, a typed contract, and a defined failure mode.

### Backend points

| Point | Contributes | Invoked |
| --- | --- | --- |
| `task.action` | An action on a task (button/command) | on demand, user-initiated |
| `task.field` | A custom field type + validation | on read/write of the field |
| `automation.trigger` | A condition source for rules | on domain event |
| `automation.action` | An effect a rule can run | on rule match |
| `event.subscriber` | A webhook consumer of domain events | after commit, async |
| `notification.channel` | A delivery target (Slack, Teams, …) | on notification fan-out |
| `validation.transition` | An extra precondition on a transition | inside the transition check |

### Frontend points

| Point | Contributes | Invoked |
| --- | --- | --- |
| `ui.task.panel` | A panel in the task drawer | on task open, lazy |
| `ui.task.badge` | A badge on cards/rows | on list render, from cached data |
| `ui.project.tab` | A tab in a project | on tab select, lazy |
| `ui.command` | An entry in the command palette | on palette open |
| `ui.settings.section` | An admin settings page | on navigation, lazy |

Adding a row to either table is an ADR trigger. The set is deliberately small:
these cover every integration in the competitive survey
([12](12-COMPETITIVE-ANALYSIS.md)) without a generic "run arbitrary code here"
escape hatch.

### `validation.transition` — the one that needs care

This point runs **inside** the transition's authorization path, so a plugin can
block work. It is included because "QA sign-off required before Done" is the most
requested tracker customization in existence, and without it every such team
forks.

It is bounded hard:
- 500 ms timeout, non-negotiable, no retry.
- **Fail-open by default** — timeout or error allows the transition and records a
  `plugin.validation.skipped` audit event.
- A workspace admin may opt a *specific* plugin into fail-closed, explicitly,
  with a warning that an outage will block work.
- The circuit breaker trips after 5 consecutive failures and stays open 60 s.

Fail-open is the default because a broken integration must not stop a team from
working. Teams with a compliance need can choose otherwise, knowingly.

## The four plugin classes

Ordered by trust required. Prefer the least-privileged that works.

### 1. Declarative — no code at all

A manifest only. Custom fields, automation rules, statuses/workflows, saved
views, badge mappings. Runs entirely inside the core interpreter; nothing
executes.

**Trust: none required.** No timeout, no sandbox, no network. Most "plugins"
should be this. Ships Phase 3a.

### 2. Remote HTTPS — request/response and webhooks

The plugin is a service the customer runs. TaskForge calls it, or delivers events
to it.

- Outbound calls signed (HMAC-SHA256 over body + timestamp + nonce), replay
  window 5 min.
- Egress allow-list per installation; no private IP ranges, no redirects
  followed off the allow-list (SSRF defense).
- Timeouts: 500 ms synchronous points, 10 s async delivery.
- Retries: exponential backoff, 6 attempts, then dead-letter.
- Per-installation quota ([21](21-API-LIMITS-AND-QUOTAS.md)).

**Trust: network + declared scopes.** No code runs on our infrastructure. This is
the workhorse class and ships Phase 3b.

### 3. Managed worker — a container we run

For plugins whose authors cannot host. An OCI image executed in an isolated
runtime with no ambient network, its own scoped token, CPU/memory caps, and a
wall-clock ceiling.

**Trust: highest.** Signed images, pinned digests, a reviewed registry.
Deliberately **deferred past v1** (ADR-016) — it triples the operational surface
(image supply chain, sandbox escape, noisy neighbours, cost attribution) for a
capability classes 1 and 2 cover for nearly every real integration. The manifest
schema reserves it so adding it later is additive.

### 4. Frontend module — sandboxed UI

Two tiers, chosen by trust:

| Tier | Isolation | For |
| --- | --- | --- |
| **Sandboxed iframe** (default) | `sandbox="allow-scripts"`, distinct origin, typed `postMessage` RPC, no ambient credentials | third-party |
| **ES module** | dynamic `import()` into the host, capability object injected | first-party / reviewed only |

**Iframe is the default and the only one open to the marketplace.** An ES module
shares the host's origin and JS context — a supply-chain compromise of that plugin
is a compromise of the whole session. That is acceptable only for code we ship.

Hard rules for both:
- **Zero bytes in the core bundle.** Loaded on demand, after first paint, never
  blocking task or board render ([42](42-FRONTEND-ARCHITECTURE.md)).
- A panel that fails to load renders an inline error in **its own panel** and
  nothing else changes.
- CSP forbids `unsafe-eval` and restricts `connect-src` to the plugin's declared
  origins.
- The host passes data in; the plugin never reads host cookies or storage.

## The manifest

```toml
[plugin]
id              = "com.example.qa-signoff"      # reverse-DNS, immutable
name            = "QA Sign-off"
version         = "1.4.0"                        # semver
contract        = "^1"                           # plugin CONTRACT range, not app version
classes         = ["declarative", "remote"]

[scopes]                                         # least privilege, admin-consented
required = ["task:read", "task:transition:validate", "project:read"]
optional = ["comment:write"]

[endpoints]
base_url        = "https://qa.example.com/taskforge"
egress_allow    = ["qa.example.com"]

[[extension]]
point           = "validation.transition"
id              = "qa-gate"
to_state        = "COMPLETED"
path            = "/validate"
timeout_ms      = 500
on_failure      = "allow"                        # admin may override to "block"

[[extension]]
point           = "ui.task.panel"
id             = "qa-panel"
title           = "QA"
tier            = "iframe"
url             = "https://qa.example.com/panel"

[data]
retention_days  = 30                             # declared, shown at consent
pii             = ["assignee_email"]             # declared, shown at consent

[compat]
min_contract    = "1.0"
max_contract    = "1.x"
```

**`contract` is versioned independently of the application** ([02](02-ARCHITECTURE.md)).
A plugin pins the extension-point contract, never the app version or the schema.
This is what lets TaskForge ship weekly without breaking the ecosystem.

## Scopes

Scopes are coarser than internal permissions on purpose — a plugin should not
need to understand the RBAC model.

| Scope | Grants |
| --- | --- |
| `task:read` | read tasks in installed projects |
| `task:write` | create/update tasks |
| `task:transition` | move tasks |
| `task:transition:validate` | participate in transition validation |
| `comment:read` / `comment:write` | comments |
| `attachment:read` | attachment metadata + signed download URLs (never raw objects) |
| `project:read` | project metadata |
| `user:read:basic` | display name + avatar only — **never** email unless separately declared |
| `webhook:subscribe` | receive events |
| `admin:settings` | own settings page |

**The ceiling** ([04](04-RBAC-AND-AUTHORIZATION.md)): a plugin's effective
permissions are the intersection of its consented scopes with the installing
admin's permissions **at install time**. A plugin can never exceed its installer.
If the installer is later downgraded, the installation is flagged for re-consent
rather than silently retaining reach.

**Data access is always mediated.** No plugin gets a database connection, a raw
object-store key, or an unscoped token. Scoped tokens are per installation, per
workspace, short-lived, and rotatable.

## Install, consent, upgrade, uninstall

**Install** shows the admin, before anything is granted: every requested scope in
plain language, every extension point and where it will appear, declared data
retention, declared PII, and the egress allow-list. Consent is recorded as an
`audit_event` with the exact manifest hash.

**Upgrade**: a version that requests **no new scopes** may auto-upgrade if the
admin enabled that. Any new scope requires fresh consent; the old version keeps
running until then. Scope escalation is never silent.

**Revocation** is immediate — tokens invalidated, in-flight calls abandoned,
circuit opened. Not deferred to a background job.

**Uninstall** is a defined lifecycle, not a delete:

1. Extension points deregister immediately; UI stops rendering them.
2. Tokens and secrets are destroyed.
3. Queued jobs for the plugin are dropped.
4. Plugin-owned data (custom field values, stored config) enters a **30-day
   grace period** — retained but inaccessible — then hard-deleted.
5. Core data the plugin *touched* (tasks, comments) is untouched. A plugin that
   wrote a comment does not get to unwrite it.

The grace period exists because "uninstall to fix a bug, reinstall" is the most
common admin action and must not destroy a quarter of custom field data.

## Failure isolation — the non-negotiable

**No plugin can fail a core request.** Concretely:

| Failure | Result |
| --- | --- |
| Plugin times out | Default outcome applied; `plugin.timeout` audit event |
| Plugin returns 5xx | Retried async; never surfaced as a core 5xx |
| Plugin returns malformed data | Rejected at the contract boundary; logged; ignored |
| Plugin exceeds quota | Throttled, admin notified; core unaffected |
| Plugin panel throws | That panel renders an error; the drawer works |
| Plugin unreachable for hours | Circuit open, events queued to the DLQ, work continues |

Every plugin interaction is **outside the core transaction**. The only synchronous
point is `validation.transition`, and it is bounded at 500 ms and fails open.

## Observability

Per installation, visible to the workspace admin — not just to operators:

- call volume, p50/p95/p99 latency, error rate, timeout rate;
- circuit breaker state and last trip;
- quota consumption against limit;
- dead-letter depth;
- last successful call per extension point.

An admin must be able to answer "is this plugin healthy, and is it slowing my
team down" without reading server logs. Plugins that degrade the experience should
be *visibly* the cause.

## Delivery

| Phase | Ships |
| --- | --- |
| **1** | Extension point registry, exercised by core panels only |
| **3a** | Declarative plugins + manifest + consent + audit |
| **3b** | Remote HTTPS: webhooks, actions, validation, notification channels |
| **3c** | Sandboxed iframe frontend panels + command registration |
| **4** | Automation trigger/action contribution, integration SDK |
| **later** | Managed workers (ADR-016), marketplace, review pipeline |

## Alternatives considered

| Option | Why not |
| --- | --- |
| **WASM plugins in-process** | Genuinely attractive in Rust — real sandbox, low latency, no network. But it puts customer code in the API process, which [01](01-ORD.md) rules out as a non-goal, and WASI's host-call surface becomes a permanent compatibility contract. Revisit for managed workers, where the sandbox is the point. |
| **Hook/filter system** (WordPress-style) | Maximum flexibility, zero contract. Every plugin couples to internals; nothing can be refactored. This is the failure mode the doc exists to prevent. |
| **Fork-and-patch** (what OrangeScrum/Redmine effectively require) | Kills upgrades. The reason customers end up two years behind on security patches. |
| **Generic "custom code" field** | An arbitrary-code-execution feature with a friendly name. |

## ADRs triggered

- **ADR-009** — Extension point registry as a closed, versioned set; core features
  render through it.
- **ADR-015** — Plugin contract versioned independently of app and schema.
- **ADR-016** — Managed-worker plugins deferred past v1; manifest reserves the slot.
- **ADR-017** — `validation.transition` fails open by default.
