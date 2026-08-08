# TaskForge — Product and System Architecture
**Status:** Initial architecture baseline  
**License target:** Apache-2.0  
**Working name:** TaskForge

## 1. Executive Summary
- TaskForge (working name) is a lightweight, plugin-enabled project and task-management platform. It starts with a simple core—projects, tasks, tags, status, state, environments, history, roles and permissions—and grows through extensions rather than permanent core complexity.
- The product is a clean implementation intended for Apache License 2.0. It must not copy OrangeScrum source, assets, templates or implementation.
- Primary architectural decisions: modular monolith, PostgreSQL system of record, explicit project-scoped RBAC, append-only activity/audit history, configurable workflows mapped to stable semantic states, isolated plugins, REST plus SSE/WebSocket updates, and a deliberately lightweight web client.

## 2. Product Principles
- Fast by default: common actions occur inline with optimistic UI and minimal navigation.
- Progressive disclosure: users initially see only essential fields; advanced fields appear when enabled.
- One work-item model: task, bug, feature, incident and request are task types, not separate incompatible entities.
- Explicit authority: every mutation is authorized on the server; hiding a button is never treated as security.
- Complete traceability: material changes create immutable history records.
- Complexity at the edges: integrations, specialized views and industry-specific features belong in plugins.

## 3. Scope and Non-Goals
- Core scope: workspaces, users, teams, projects, project environments, tasks, subtasks, dependencies, tags, milestones, configurable statuses, stable states, comments, attachments, saved views, notifications, roles, permissions, activity, audit, automations and plugins.
- Not in first release: CRM, payroll, invoicing, office document editing, chat, video meetings, whiteboards, arbitrary custom server code in the core process, complex portfolio finance, or microservices for every module.

## 4. Domain Model
- Workspace is the tenant boundary. A user joins through a workspace membership and may belong to multiple workspaces.
- Project is the primary collaboration boundary. Projects can be private, team-visible or workspace-visible.
- Task is the universal work item. Important fields include key, title, description, type, priority, status, state, reporter, assignees, project, environment, milestone, parent, dates, tags, version, archive and deletion timestamps.
- Environment is an optional project context such as Development, QA, Staging, Production, Customer UAT or Region EU.
- Tag is a reusable workspace- or project-scoped label. Tags are many-to-many with tasks.
- History is an append-only stream of user-readable activity. Audit is a more security-focused stream with additional request and authentication metadata.

## 5. State, Status and Workflow
- State is a stable semantic category used by APIs, analytics and plugins: BACKLOG, PLANNED, ACTIVE, COMPLETED and CANCELED.
- Status is configurable and maps to one state. Examples: Ready for Development maps to PLANNED; In Progress and Code Review map to ACTIVE; Done maps to COMPLETED.
- A workflow contains statuses and allowed transitions. Every transition can require a permission, validation rules, mandatory fields, dependency checks or automation hooks.
- Default workflow: Backlog → Todo → In Progress → Done, with Blocked as an ACTIVE status and Canceled as a terminal CANCELED status.
- Closing a task requires both task.close permission and a valid transition to a COMPLETED status. Reopening requires task.reopen and a permitted transition out of a terminal status.

## 6. Roles and Permission Architecture
- Roles are administrator-defined collections of stable permissions. Built-in templates are Owner, Administrator, Project Manager, Member and Guest. Administrators can clone templates and create custom roles such as QA Reviewer or Release Manager.
- A role assignment targets a principal—user, team, group or service account—and a scope: WORKSPACE, TEAM, PROJECT, ENVIRONMENT, with TASK reserved for exceptional sharing.
- A person can be Project Manager in one project and Guest in another. Project-level role assignment is therefore fundamental.
- Permission examples: project.create, project.update, project.member.manage, project.role.assign, project.workflow.manage, task.create, task.update, task.assign, task.move, task.close, task.reopen, task.delete, task.comment, task.history.read, tag.manage, plugin.install and automation.manage.
- Authorization evaluates actor, permission, resource, workspace and contextual constraints. Initial constraint support should be deliberately small: assignee-only, reporter-only, project member, environment, ownership and external-user restrictions.
- Privilege escalation protections: users cannot delegate permissions they lack authority to grant; project managers cannot grant workspace roles; the last owner cannot be removed; plugin permissions require explicit consent; role changes are audited.

## 7. Permission Examples
- Create task: actor must have task.create in the target project.
- Assign task: actor must have task.assign, and the assignee must be an eligible project member unless an administrator overrides policy.
- Close task: actor must have task.close and workflow validation must allow the transition.
- Reopen task: actor must have task.reopen and a valid reverse transition.
- Update own assigned tasks only: assign a role with task.update plus an assignee-is-actor constraint.
- Manage project roles: require project.role.assign and block assignment of permissions beyond the delegator’s grant ceiling.

## 8. UX and Information Architecture
- Permanent navigation: Home, My Work, Inbox, Search, Teams/Projects, Saved Views and Create. Administration appears only when authorized.
- Project views in core: Overview, Board, List and Activity. Calendar, Timeline, Workload, Releases and specialized reports can arrive later or through plugins.
- Clicking a task opens a side drawer while preserving board/list context. A dedicated full-page route remains available for deep links.
- Task creation initially asks only for title, project, status and optional assignee. Description, due date, tags, environment, milestone and custom fields are progressively disclosed.
- My Work aggregates assignments across accessible projects into Today, Overdue, Upcoming, Blocked and Recently Completed.
- Command palette handles create, navigate, assign, transition, search and plugin commands without bloating permanent navigation.

## 9. Lightweight Client Requirements
- The client must remain a thin interaction layer, not a second application server. Business rules, authorization, filtering, sorting and workflow validation belong on the backend.
- Initial authenticated shell target: approximately 150–200 KB compressed JavaScript where practical, excluding optional editor and plugin chunks. Exact budgets should be enforced in CI and adjusted only through architecture review.
- Route-level and feature-level code splitting is mandatory. Calendar, timeline, reports, admin screens, rich text, charts and plugin UI load only when opened.
- Use server-side cursor pagination, filtering, sorting and search. Never download an entire large project to render a board or list.
- Virtualize long lists, boards and activity streams. Render only visible rows/cards plus a small overscan window.
- Use SSE for most server-to-client updates; WebSocket is optional where bidirectional low-latency interaction is genuinely required. Batch and coalesce events.
- Use optimistic updates for common mutations, with version-aware rollback on conflict. Avoid broad client state stores; server state belongs in TanStack Query and local transient UI state remains local.
- No heavy office editor, canvas framework, embedded analytics runtime or large component suite in the core bundle. Rich text should be loaded only when the user edits content.
- Prefer native CSS, CSS variables and accessible headless primitives. Avoid shipping multiple icon libraries and duplicate date/time packages.
- Performance targets: first usable shell under 2.5 seconds on a mid-tier laptop and constrained broadband; p75 interaction latency under 100 ms for local UI actions; no long tasks over 200 ms during common board/list interaction; smooth 60 fps dragging where hardware permits.
- Offline-first is not required initially. A small draft cache and retry queue are acceptable, but do not ship a large synchronization engine until product need is proven.

## 10. System Architecture
- Start with a modular monolith. Logical modules: identity, workspace, authorization, teams, projects, workflows, tasks, comments, attachments, activity, notifications, search, saved views, automation, plugins, integrations, audit and administration.
- Modules must use internal application interfaces and domain events rather than directly reading another module’s tables.
- Recommended backend: Java LTS, Spring Boot, Spring Security, PostgreSQL, Flyway, Redis, S3-compatible storage, OpenTelemetry, Micrometer, Testcontainers and Gradle.
- Recommended frontend: React, TypeScript, Vite, TanStack Query/Router, accessible headless UI primitives, dnd-kit, a lightweight lazy-loaded rich text editor, list virtualization and Playwright.
- PostgreSQL is the system of record. Redis supports rate limits, short-lived caches and fan-out. Object storage holds attachments. Search begins with PostgreSQL full-text and trigram indexes; external search is introduced only when scale proves it necessary.

## 11. API and Concurrency
- Use versioned REST APIs for commands and reads, with SSE or WebSocket for live updates.
- Status change uses a dedicated transition command rather than arbitrary status-field mutation.
- Use cursor pagination, idempotency keys for retried creates, structured errors, correlation identifiers and OpenAPI contracts.
- Use optimistic concurrency through a numeric aggregate version and If-Match headers. Return HTTP 409 with current representation and changed-field information on conflict.
- Example endpoints: POST /api/v1/projects/{id}/tasks, PATCH /api/v1/tasks/{id}, POST /api/v1/tasks/{id}/transitions, POST /api/v1/tasks/{id}/comments, GET /api/v1/tasks/{id}/activity, POST /api/v1/roles, POST /api/v1/role-assignments and GET /api/v1/permissions/effective.

## 12. Events, History and Audit
- Domain mutation and outbox event are committed in one PostgreSQL transaction. A dispatcher publishes to live-update channels, notification handlers, webhook delivery, plugin workers, search projections and analytics.
- Important events include task.created, task.updated, task.status.changed, task.closed, task.reopened, task.assigned, task.tag.added, comment.created, attachment.added, role.assigned, workflow.updated and plugin.permission.granted.
- Each event includes event ID, tenant, aggregate, event type, actor, source, timestamp, request ID, correlation ID, before/after values and metadata.
- Activity is optimized for people. Audit is optimized for investigation and compliance and may have separate retention and access policy.

## 13. Plugin Architecture
- Plugins are installed per workspace and may contribute task panels, task actions, badges, project tabs, commands, custom field types, automation triggers/actions, notification channels and webhook consumers.
- Core plugin forms: declarative plugin, remote HTTPS integration, isolated managed worker/container and sandboxed frontend extension.
- Plugins receive explicit scopes and never direct core database access. Installation requires administrator consent and records requested permissions, configuration schema, compatibility range and data-retention declaration.
- Security controls: signed packages/manifests, per-installation secrets, timeouts, circuit breakers, quotas, audit logs, egress restrictions, compatibility checks, permission revocation and uninstall cleanup.
- Frontend plugins must load on demand. They cannot increase the default application bundle or block project/task rendering.

## 14. Data Model Overview
- Primary tables: workspace, user_account, workspace_membership, role, permission, role_permission, role_assignment, team, team_membership, project, project_membership, project_environment, workflow, workflow_status, workflow_transition, task, task_assignee, tag, task_tag, task_dependency, milestone, comment, attachment, activity_event, audit_event, saved_view, notification, automation_rule, plugin_installation, service_account and api_token.
- All tenant records contain workspace_id, created_at, created_by, updated_at, updated_by and version where mutable.
- Use UUIDv7 or another sortable globally unique identifier. Give tasks a human-readable project key and monotonically allocated project task number.
- Use normalized tables for the core. JSONB is acceptable for plugin metadata and controlled custom-field values only with schemas, validation and appropriate indexes.

## 15. Security and Tenancy
- Workspace ID is mandatory in every tenant query, cache key, object-store key, search document and background-job context.
- WebSocket/SSE subscriptions revalidate membership. Permission caches are short lived and never the sole authority.
- Baseline: OIDC/SAML readiness, optional local auth, MFA, secure cookies, CSRF protection, CSP, encrypted secrets, rate limits, dependency scanning, container scanning, SBOM, signed releases, audit export and restore drills.
- File upload uses pre-signed URLs, content-type verification, size limits, malware scanning and a completion handshake before attachment visibility.

## 16. Deployment and Observability
- Developer profile: application, PostgreSQL, Redis, MinIO and mail catcher in Docker Compose.
- Small self-hosted profile: reverse proxy, application, worker, PostgreSQL, Redis and S3/MinIO.
- Scaled profile: stateless application replicas, worker pools, managed PostgreSQL, Redis HA, external object storage and optional search cluster.
- Observe OpenTelemetry traces, structured logs, API latency/error metrics, database pool, outbox lag, delivery failures, permission denials, transition failures, attachment backlog and frontend Web Vitals.

## 17. Delivery Phases
- Phase 0 — foundation: repository, Apache-2.0 license, ADR process, CI, coding standards, threat model, design tokens and performance budgets.
- Phase 1 — usable core: authentication, workspace, membership, projects, task CRUD, tags, comments, attachments, default workflow, simple roles, My Work, board, list and activity.
- Phase 2 — administration: custom roles, project-scoped assignments, permission simulator, custom statuses/workflows, environments, milestones, saved views, audit and exports.
- Phase 3 — extension platform: manifests, scopes, plugin installation, webhooks, task panels/actions, command registration, isolated workers and integration SDK.
- Phase 4 — advanced productivity: automations, calendar/timeline plugins, reporting projections, SSO enterprise controls and optional external search.
- Every phase must satisfy client bundle, API latency, accessibility, migration, backup and security gates before release.

## 18. Required Architecture Decisions Before Coding
- Approve product vocabulary and exact distinction between state and status.
- Approve role assignment scopes and initial constrained-permission set.
- Decide whether multiple assignees are allowed in v1; recommended: yes, with one optional primary owner.
- Decide whether environments are single-select or multi-select on tasks; recommended v1: single optional environment.
- Approve task key allocation and deletion/retention policy.
- Approve frontend bundle budgets and browser support matrix.
- Approve plugin trust model and whether managed server plugins are postponed until after declarative/remote plugins.
- Approve audit retention defaults and privacy treatment of IP/device metadata.
- Create ADRs for modular monolith, PostgreSQL, RBAC, transactional outbox, plugin isolation and lightweight-client constraints.
