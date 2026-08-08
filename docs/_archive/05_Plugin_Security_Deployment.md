# TaskForge — 05 Plugin Security Deployment

## 13. Plugin Architecture
- Plugins are installed per workspace and may contribute task panels, task actions, badges, project tabs, commands, custom field types, automation triggers/actions, notification channels and webhook consumers.
- Core plugin forms: declarative plugin, remote HTTPS integration, isolated managed worker/container and sandboxed frontend extension.
- Plugins receive explicit scopes and never direct core database access. Installation requires administrator consent and records requested permissions, configuration schema, compatibility range and data-retention declaration.
- Security controls: signed packages/manifests, per-installation secrets, timeouts, circuit breakers, quotas, audit logs, egress restrictions, compatibility checks, permission revocation and uninstall cleanup.
- Frontend plugins must load on demand. They cannot increase the default application bundle or block project/task rendering.

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
