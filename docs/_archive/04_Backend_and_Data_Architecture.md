# TaskForge — 04 Backend and Data Architecture

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

## 14. Data Model Overview
- Primary tables: workspace, user_account, workspace_membership, role, permission, role_permission, role_assignment, team, team_membership, project, project_membership, project_environment, workflow, workflow_status, workflow_transition, task, task_assignee, tag, task_tag, task_dependency, milestone, comment, attachment, activity_event, audit_event, saved_view, notification, automation_rule, plugin_installation, service_account and api_token.
- All tenant records contain workspace_id, created_at, created_by, updated_at, updated_by and version where mutable.
- Use UUIDv7 or another sortable globally unique identifier. Give tasks a human-readable project key and monotonically allocated project task number.
- Use normalized tables for the core. JSONB is acceptable for plugin metadata and controlled custom-field values only with schemas, validation and appropriate indexes.
