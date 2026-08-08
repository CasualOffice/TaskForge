# Security Policy

## Reporting

**Do not open a public issue** for a suspected vulnerability.

Use [GitHub private vulnerability reporting](https://github.com/CasualOffice/taskforge/security/advisories/new)
for this repository. Include:

- affected revision or release;
- affected subsystem (authorization, tenancy, auth/session, attachments, plugin
  plane, API);
- impact and a realistic attack path;
- a minimal reproduction, **without confidential customer data**;
- whether active exploitation is known;
- how you would like to be credited.

If you cannot use GitHub, contact a maintainer privately and we will arrange a
channel.

## Response

| Stage | Target |
| --- | --- |
| Acknowledgement | 48 hours |
| Initial assessment and severity | 5 business days |
| Fix or documented mitigation | severity-dependent; critical issues take priority over all other work |
| Coordinated disclosure | agreed with the reporter, default 90 days |

Security fixes ship **out of band**, bypassing the normal release cadence — but
not the CI gates. A fix that cannot pass the gates is not ready.

## What we consider a vulnerability

Ranked by how seriously we treat it:

1. **Cross-tenant data access** — any path by which one workspace reaches
   another's data. This is our most severe class
   ([docs/32](docs/32-TENANCY-AND-ISOLATION.md)).
2. **Privilege escalation** — obtaining a permission that was not granted,
   including through role assignment, plugin installation, automation `run_as`,
   or API token scoping ([docs/04](docs/04-RBAC-AND-AUTHORIZATION.md)).
3. **Authentication bypass** — session forgery, token forgery, SSO assertion
   handling, MFA bypass, or a revocation that does not revoke
   ([docs/40](docs/40-IDENTITY-AUTH-AND-SESSION.md)).
4. **Audit integrity** — any way to modify or suppress an activity or audit
   record. These tables are append-only by database grant; a path around that is
   a vulnerability ([docs/25](docs/25-EVENTS-OUTBOX-AND-AUDIT.md)).
5. **Plugin escape** — a plugin exceeding its consented scopes, reaching the core
   database, executing in the API process, or breaking out of the frontend
   sandbox ([docs/34](docs/34-PLUGIN-AND-EXTENSION-ARCHITECTURE.md)).
6. **Stored or reflected XSS**, including via attachments served from the wrong
   origin ([docs/28](docs/28-ATTACHMENT-PIPELINE.md)).
7. **SSRF** through webhook or plugin egress.
8. **Injection** — SQL, or command injection anywhere.
9. **Account enumeration** through login, password reset, invitation, or user
   search. We treat this as a real finding, not an informational one.
10. **Denial of service** through an unbounded input. Every input is supposed to
    be bounded ([docs/21](docs/21-API-LIMITS-AND-QUOTAS.md)); an unbounded one is
    a bug.

## What we do not consider a vulnerability

- Missing security headers on an endpoint that serves no content.
- Rate limits you consider too generous, absent a demonstrated impact.
- Self-XSS requiring the victim to paste code into a console.
- Vulnerabilities in a dependency with no reachable path in TaskForge (report
  them anyway — we will assess reachability and update `deny.toml`).
- Findings from an automated scanner with no demonstrated exploit path.
- Social engineering of maintainers or users.
- A permission model limitation that is **documented as a deliberate trade** —
  most notably the absence of deny rules ([docs/04](docs/04-RBAC-AND-AUTHORIZATION.md)).
  If you believe the trade is wrong, that is a design discussion, and a welcome
  one; it is not a vulnerability report.

## Supported versions

TaskForge has not made its first release. Until then, **only `main` is
supported.** A support window for released versions will be published with the
first release and recorded in [docs/18-SUPPORT-MATRIX.md](docs/18-SUPPORT-MATRIX.md).

## Deployment security

If you self-host, these are your responsibility and are documented in
[docs/48-DEPLOYMENT-PROFILES.md](docs/48-DEPLOYMENT-PROFILES.md):

- **`TF_ATTACHMENT_ORIGIN` must differ from `TF_PUBLIC_URL`.** Sharing the origin
  defeats attachment isolation. The application refuses to start if they match.
- Keep `TF_SECRET_KEY` out of version control and rotate it on suspected exposure.
- Terminate TLS in front of the application; enable HSTS.
- Restrict database network access to the application.
- Configure plugin egress allow-lists narrowly.
- Test your restore procedure. A backup that has never been restored is a
  hypothesis about a file.

## Our practices

- `unsafe_code = "forbid"` workspace-wide.
- All SQL is compile-checked and parameterized; the filter compiler is
  property-tested to emit no user-derived SQL strings.
- Secret scanning, SAST, dependency advisories, and container scanning gate every
  build ([docs/15](docs/15-CI-AND-RELEASE-GATES.md)).
- Escalation, cross-tenant, and enumeration test suites run on every PR.
- Fuzzing on the filter grammar and the plugin manifest parser.
- Threat model reviewed at every phase gate
  ([docs/07](docs/07-QUALITY-SECURITY-AND-COMPATIBILITY.md)).
- SBOM published per release; releases signed.

## Credit

We credit reporters in the advisory and the changelog unless you prefer
otherwise. We do not currently operate a paid bounty program.
