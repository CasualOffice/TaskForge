# 52 — Deployment Guide

The operator walkthrough. [48](48-DEPLOYMENT-PROFILES.md) is the *architecture*
of the three profiles; this is *how to run one*.

> **Status: Phase 0.** The image builds, runs, and ships the migrations, but the
> binaries are scaffolds — there is no API yet. Everything below is the
> committed procedure; the steps marked **⧗ not yet executable** become live in
> Phase 1. Nothing here is aspirational about *how* it will work, only about
> *when*.

## Quick start — self-hosted, single node

```sh
git clone https://github.com/CasualOffice/TaskForge && cd TaskForge

cp deploy/.env.example deploy/.env
$EDITOR deploy/.env            # every CHANGE_ME must change

docker compose -f deploy/docker-compose.yml --env-file deploy/.env up -d
docker compose -f deploy/docker-compose.yml logs -f api
```

That is one binary plus PostgreSQL. No Redis, no object storage, no separate
worker. Keeping this profile genuinely supported is a **constraint on the
architecture**, not a convenience — a design that required a message broker
would have eliminated it ([48](48-DEPLOYMENT-PROFILES.md)).

**Capacity:** roughly 50 users and 100k tasks on 2 vCPU / 4 GB.

## Before you start: three settings that are not optional

The stack refuses to start without them. A deployment that starts
misconfigured is worse than one that does not start.

### `TF_SECRET_KEY`

```sh
openssl rand -base64 48
```

Signs sessions and cookies. Rotating it invalidates every session — which is
the correct response to suspected exposure, not a reason to avoid rotating.

### `TF_ATTACHMENT_ORIGIN` must be a different **host**, not a different path

```
TF_PUBLIC_URL=https://tasks.example.com
TF_ATTACHMENT_ORIGIN=https://files.tasks.example.com     # ✅ different origin
TF_ATTACHMENT_ORIGIN=https://tasks.example.com/files     # ❌ same origin
```

User-uploaded files are served from this origin. If it matches the application
origin, a stored HTML or SVG attachment executes **with access to session
cookies** — every other attachment control (magic-byte sniffing, malware
scanning, `Content-Disposition`) is defence in depth behind this one
([28](28-ATTACHMENT-PIPELINE.md)). The application refuses to start if they
match.

### `TASKFORGE_DB_PASSWORD` — the application is not the database owner

Two roles exist deliberately:

| Role | Used by | Why |
| --- | --- | --- |
| `POSTGRES_USER` (owner) | migrations, retention worker | needs DDL |
| `taskforge_app` | **the application** | ordinary, non-superuser |

**A superuser bypasses row-level security unconditionally.** `FORCE ROW LEVEL
SECURITY` forces policies for the table *owner*; it cannot constrain a
superuser. The same applies to append-only history: `REVOKE UPDATE, DELETE ON
audit_event` has no effect on a superuser.

So if the application connects as the owner, **tenant isolation and audit
immutability are both silently inert** — the policies exist, the tests pass in
isolation, and nothing enforces anything. `deploy/init-app-role.sh` creates the
role on first initialization; migration 0012 hardens it
(`NOSUPERUSER NOBYPASSRLS`). See [32](32-TENANCY-AND-ISOLATION.md).

This was a real bug in this repository, found by running the schema rather than
reading it. It is called out here because a hand-rolled deployment can
reintroduce it in one line.

## Building the image yourself

```sh
docker build -t taskforge:local .
docker run --rm --entrypoint /usr/local/bin/taskforge-api taskforge:local
```

| Property | Value |
| --- | --- |
| Base | `gcr.io/distroless/cc-debian12:nonroot` |
| Size | ~49 MB (target < 100 MB, [30](30-PERFORMANCE-AND-CAPACITY-TARGETS.md)) |
| User | `65532:65532`, declared explicitly so a base-image change cannot silently promote it to root |
| Contains | `taskforge-api`, `taskforge-worker`, `/app/migrations` |

**Distroless, not `scratch`.** TLS roots, timezone data, and the ability to get
a debugger into a container during an incident are worth the megabytes
([19](19-WORKSPACE-SCAFFOLD-DESIGN.md)).

**Migrations ship inside the image** so the schema version and the code version
that expects it cannot disagree.

## Reverse proxy and TLS

TaskForge does not terminate TLS. Put a proxy in front.

### The database connection is NOT encrypted (D-050)

**PostgreSQL must be reachable only over a trusted network** — the same host, or
a private subnet you control. TaskForge connects to it in the clear.

This is a decision, not an oversight, and it has a cost worth stating plainly:
**a managed PostgreSQL reached across a public network is not a supported
deployment today.** If your database provider requires `sslmode=require`, this
release cannot connect to it.

Why: enabling TLS in `sqlx` pulls in `webpki-roots`, Mozilla's CA bundle,
licensed `CDLA-Permissive-2.0` — which `deny.toml` does not allow. Widening the
licence allow-list is a policy decision, and it was made deliberately in the
other direction for now: nothing in the product yet connects to a remote
database, so the choice was between adding a licence obligation for a capability
nobody uses and documenting a constraint everybody's current deployment already
satisfies.

**What holds it:** the `dependency-policy` CI job. Turning TLS on is not a
one-line feature flag — it fails `cargo deny check licenses` with a named
licence, which is the point. The decision gets revisited when a deployment
needs it, by someone reading this section, rather than smuggled in beside an
unrelated change.

The single-node compose profile satisfies this by construction: `postgres` is
declared with `expose`, not `ports`, so it is reachable from the application
container and from nowhere else. `scripts/verify-deployment.sh` asserts that.

```nginx
server {
    listen 443 ssl http2;
    server_name tasks.example.com;

    add_header Strict-Transport-Security "max-age=31536000; includeSubDomains" always;

    location / {
        proxy_pass http://127.0.0.1:8080;
        proxy_set_header Host              $host;
        proxy_set_header X-Forwarded-For   $proxy_add_x_forwarded_for;
        proxy_set_header X-Forwarded-Proto $scheme;

        # Live updates are SSE. Without these three lines the stream is buffered
        # and the UI appears to freeze — the most common proxy misconfiguration
        # for this application (docs/05 §Live updates).
        proxy_buffering    off;
        proxy_cache        off;
        proxy_read_timeout 1h;
    }
}

server {
    listen 443 ssl http2;
    server_name files.tasks.example.com;   # separate origin — see above
    location / { proxy_pass http://127.0.0.1:8080; proxy_set_header Host $host; }
}
```

## Upgrading

```sh
docker compose -f deploy/docker-compose.yml pull
docker compose -f deploy/docker-compose.yml up -d
```

Migrations run before the new code serves traffic. They are forward-only and
follow expand → migrate → contract ([22](22-DATABASE-SCHEMA.md)), which is what
makes the previous version still able to run against the new schema — and
therefore what makes rollback possible.

**Rollback** is supported to the immediately previous version:

```sh
TASKFORGE_IMAGE=ghcr.io/casualoffice/taskforge:<previous> \
  docker compose -f deploy/docker-compose.yml up -d
```

Beyond one version, restore from backup. Contracting migrations land in a
*later* release than the expand that preceded them, which is exactly the
discipline that bounds rollback to one version — and why skipping it is not a
shortcut.

## Backups

```sh
# Database
docker compose -f deploy/docker-compose.yml exec -T postgres \
  pg_dump -U taskforge_owner -Fc taskforge > taskforge-$(date +%F).dump

# Attachments — the database alone is not a backup
docker run --rm -v taskforge_attachments:/data -v "$PWD":/backup alpine \
  tar czf /backup/attachments-$(date +%F).tar.gz -C /data .
```

**Back up both, always.** A database restore without the matching attachments
produces rows pointing at nothing.

### Restore drill

```sh
createdb taskforge_restore_test
pg_restore -d taskforge_restore_test taskforge-2026-08-08.dump
psql -d taskforge_restore_test -c 'SELECT count(*) FROM task;'
```

Run it on a schedule, not after an incident. **A backup that has never been
restored is a hypothesis about a file** ([15](15-CI-AND-RELEASE-GATES.md)).

| Profile | Method | RPO | RTO |
| --- | --- | --- | --- |
| Single node | nightly `pg_dump` + volume tar | 24 h | hours |
| Small | WAL archiving + PITR | < 5 min | < 1 h |
| Scaled | continuous + cross-region | < 1 min | < 30 min |

## Verifying a deployment is actually secure

Run these after any install or migration. Each checks something that fails
**silently**.

```sh
psql "$DATABASE_URL" -c "
  SELECT rolname, rolsuper, rolbypassrls FROM pg_roles WHERE rolname='taskforge_app';"
```
Expect `f | f`. If either is `t`, RLS and audit immutability are inert.

```sh
psql "$OWNER_URL" -c "
  SELECT count(*) FROM pg_class c JOIN pg_namespace n ON n.oid=c.relnamespace
   WHERE n.nspname='public' AND c.relrowsecurity;"
```
Expect 30.

```sh
psql "$DATABASE_URL" -c "UPDATE audit_event SET event_type='x';"
```
Expect `ERROR: permission denied`. If it succeeds, history is rewritable.

The repository automates all of this:

```sh
./scripts/verify-schema.sh          # 8 structural + 6 behavioural assertions
```

## Scaling past one node

Move to Profile 2 ([48](48-DEPLOYMENT-PROFILES.md)) when a single node stops
being enough. What changes:

| Change | Why |
| --- | --- |
| Redis becomes **required** | rate limits and SSE fan-out need shared state across ≥ 2 API instances |
| Worker becomes its own container | `TF_WORKER_EMBEDDED=false`, so a bulk import cannot compete with request handling for CPU |
| Object storage replaces the filesystem | `TF_STORAGE_BACKEND=s3` |
| PostgreSQL gets a replica | reports read from it; **never** the write path — replica lag would make a just-created task vanish |

The attachment pipeline is **identical** across profiles — same handshake, same
scan step, same invisibility-until-committed rule. A deployment profile must
never change the security model, or the small profile becomes the insecure one.

## Troubleshooting

| Symptom | Cause | Fix |
| --- | --- | --- |
| Refuses to start, names a variable | required config missing | set it; this is intended |
| "attachment origin must differ" | `TF_ATTACHMENT_ORIGIN` == `TF_PUBLIC_URL` | use a separate host |
| Live updates never arrive | proxy buffering SSE | `proxy_buffering off` |
| Everything visible across workspaces | app connected as owner/superuser | fix `DATABASE_URL` to use `taskforge_app`; verify with the query above |
| `permission denied for table audit_event` | **working as designed** — history is append-only | do not grant the privilege |
| Migration hangs on upgrade | lock contention | check `pg_stat_activity`; a migration exceeding its budget aborts by design |

For live incidents, use the runbooks: [50](50-RUNBOOKS.md).

## What is not supported

- **Kubernetes as a requirement.** Manifests are provided; plain Docker or
  systemd is fully supported. Requiring an orchestrator would exclude exactly
  the self-hosters this product is for.
- **Multi-region active-active.** Not designed; not promised
  ([18](18-SUPPORT-MATRIX.md)).
- **Running the application as the database owner.** Covered above — it is not a
  configuration choice, it is a broken deployment.
