# 48 — Deployment Profiles

Three supported shapes. The security model is **identical** across all three —
a smaller deployment is smaller, never weaker ([28](28-ATTACHMENT-PIPELINE.md)).

## Profile 1 — Single node

**For:** self-hosters, small teams, evaluation. The profile that makes the
Apache-2.0 promise real.

```
┌──────────────────────────────────────┐
│  reverse proxy (TLS)                 │
├──────────────────────────────────────┤
│  casual-task-api    (embedded worker)│
├──────────────────────────────────────┤
│  PostgreSQL 16                       │
├──────────────────────────────────────┤
│  filesystem (attachments)            │
└──────────────────────────────────────┘
```

**One binary + PostgreSQL.** No Redis, no S3, no message broker, no separate
worker process.

This is a design constraint, not a convenience, and it shapes the architecture:

| Component | Optional because |
| --- | --- |
| Redis | rate limits and caches fall back to in-process (moka); SSE fan-out is single-instance |
| S3 | the object store is a trait; a filesystem backend implements it |
| Worker process | the worker runs as an embedded task in the API binary |
| Message broker | the outbox is a PostgreSQL table ([25](25-EVENTS-OUTBOX-AND-AUDIT.md)) |

Every one of those was a deliberate choice made *because* of this profile. A
design requiring Kafka would have eliminated it.

**Capacity:** ~50 users, ~100k tasks on 2 vCPU / 4 GB.
**Backup:** `pg_dump` + the attachment directory. Documented and drilled.

## Profile 2 — Small production

**For:** a real team, self-hosted or single-tenant cloud.

```
   reverse proxy / LB
        │
   ┌────┴─────┐
   │ api × 2  │      worker × 1      (separate binaries)
   └────┬─────┘           │
        └──────┬──────────┘
        PostgreSQL (primary + streaming replica)
        Redis (rate limits, SSE fan-out, cache)
        S3 / MinIO
```

Redis becomes required at ≥ 2 API instances — rate limits and SSE fan-out need
shared state. The worker splits out so a long import cannot compete with request
handling for CPU.

**Capacity:** ~500 users, ~2M tasks — the reference corpus
([30](30-PERFORMANCE-AND-CAPACITY-TARGETS.md)).
**Backup:** WAL archiving + PITR; object-store versioning.

## Profile 3 — Scaled

**For:** multi-tenant hosted.

```
   CDN → LB → api × N (autoscaled, stateless)
                 │
        ┌────────┼─────────────┬──────────────┐
   managed PG    Redis HA   object storage   (optional search cluster)
   + replicas
                 │
        worker pools, sharded by role:
          dispatch · notify · webhook · scan · projection · automation
```

- API instances are stateless; sessions and rate limits are in Redis/PostgreSQL.
- Worker pools are separated **by role**, so a webhook backlog cannot starve
  outbox dispatch, and each scales independently.
- Read replicas serve reports and analytics — never the write path, because
  replica lag would produce a task that vanishes right after being created.
- Outbox dispatch shards by `workspace_id` hash beyond ~10,000 events/s.

**Capacity:** the design ceiling in [30](30-PERFORMANCE-AND-CAPACITY-TARGETS.md).

## Configuration

Environment variables, twelve-factor, with documented defaults for every key.

```
DATABASE_URL                 required
TF_BIND_ADDR                 0.0.0.0:8080
TF_PUBLIC_URL                required — used in emails and OIDC redirects
TF_SECRET_KEY                required — session/cookie signing
TF_STORAGE_BACKEND           fs | s3            (default fs)
TF_STORAGE_PATH              ./data/attachments
TF_S3_*                      endpoint, bucket, region, credentials
TF_ATTACHMENT_ORIGIN         required in prod — the separate user-content origin
TF_REDIS_URL                 optional; required with >1 api instance
TF_WORKER_EMBEDDED           true | false       (default true)
TF_SMTP_*                    host, port, user, pass, from
TF_OIDC_*                    per-workspace in DB; these are bootstrap defaults
TF_LIMITS_*                  every limit in doc 21 is a key
TF_LOG_FORMAT                json | pretty
TF_OTEL_ENDPOINT             optional
```

**Startup validation fails fast and specifically.** Missing `TF_SECRET_KEY`
stops the process with a message naming the variable and what it is for — not a
panic 200 ms into the first request. A misconfigured deployment must not start.

`TF_ATTACHMENT_ORIGIN` is required in production and refuses to equal
`TF_PUBLIC_URL`, because sharing the origin defeats the attachment isolation
control ([28](28-ATTACHMENT-PIPELINE.md)).

## Migrations on deploy

- Run **before** new application code accepts traffic.
- Forward-only, expand → migrate → contract ([22](22-DATABASE-SCHEMA.md)), so the
  previous version still runs against the new schema — which is what makes
  rollback possible.
- Advisory-locked, so concurrent instances do not race.
- Timed; a migration exceeding its budget aborts rather than locking `task`
  during business hours.
- The single-node profile runs them automatically at startup; larger profiles run
  them as a discrete step.

## Zero-downtime deploy

1. Migrate (expand).
2. Roll new instances in; old and new run concurrently against one schema.
3. Drain old instances — SSE streams are closed with a reconnect hint so clients
   resume with `Last-Event-ID` rather than losing updates.
4. Contract in a **later** release, never the same one.

Step 4 is the discipline that makes step 3 safe. Contracting in the same release
means a rollback lands on a schema that no longer supports it.

## Backups and disaster recovery

| Profile | Backup | RPO | RTO |
| --- | --- | --- | --- |
| Single node | nightly `pg_dump` + files | 24 h | hours |
| Small | WAL archiving + PITR | < 5 min | < 1 h |
| Scaled | continuous + cross-region | < 1 min | < 30 min |

**Restore is drilled every phase** ([15](15-CI-AND-RELEASE-GATES.md)). A backup
that has never been restored is a hypothesis about a file.

Object storage is backed up independently — a database restore without the
matching attachments produces rows pointing at nothing.

## What is deliberately not supported

- **Multi-region active-active.** Requires a residency and conflict design that
  does not exist yet ([08](08-ADR-REGISTER.md) pending).
- **Sharded PostgreSQL.** Not needed below the design ceiling; adding it early
  would cost every query a shard key for no present benefit.
- **Kubernetes as a requirement.** Manifests are provided; a plain container
  runtime, or systemd, is fully supported. Requiring an orchestrator would
  exclude exactly the self-hosters this product is for.
