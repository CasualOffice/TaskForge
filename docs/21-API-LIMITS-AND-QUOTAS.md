# 21 — API Limits & Quotas

Every bound on every input. Nothing user-supplied is unbounded — an unbounded
input is a denial-of-service vector and a latency cliff waiting for the first
customer big enough to find it.

Limits are enforced **at the edge**, before a handler runs, and every rejection
names the exceeded limit with a `TF-*` code ([20](20-ERROR-CODE-REGISTRY.md)) —
never a bare `400`.

## Request-level

| Limit | Value | Code |
| --- | --- | --- |
| Request body | 1 MB (JSON) | `TF-LIM-0005` |
| URL length | 8 KB | `TF-LIM-0005` |
| Header count / size | 100 / 16 KB | — |
| Request timeout | 30 s | `TF-SYS-0005` |
| Statement timeout | 5 s (2 s for search) | `TF-QRY-0009` |
| Transaction timeout | 10 s | `TF-SYS-0005` |
| Concurrent requests per actor | 20 | `TF-LIM-0002` |
| JSON nesting depth | 32 | `TF-VAL-0001` |

## Field limits

| Field | Limit |
| --- | --- |
| Task title | 512 chars |
| Task description | 64 KB |
| Comment body | 64 KB |
| Project name | 255 chars |
| Project key | 2–10 chars, `^[A-Z][A-Z0-9]{1,9}$` |
| Tag name | 64 chars |
| Role name | 128 chars |
| Saved view name | 128 chars |
| Assignees per task | 20 |
| Tags per task | 50 |
| Dependencies per task | 100 |
| Mentions per comment | 50 |
| Dependency graph depth (check) | 64 hops |

## Query limits

From [26](26-SEARCH-INDEXING-AND-QUERY.md) and
[27](27-FILTER-AND-SAVED-VIEW-DSL.md):

| Limit | Value | Code |
| --- | --- | --- |
| Page size | 100 (default 50) | `TF-QRY-0007` |
| Filter clauses | 32 | `TF-QRY-0004` |
| Filter nesting depth | 4 | `TF-QRY-0005` |
| `IN` list length | 100 | `TF-QRY-0004` |
| Sort fields | 3 | `TF-QRY-0002` |
| Search term length | 256 chars | `TF-QRY-0008` |
| Count cap (`include=count`) | 10,000 | — (flagged in response) |
| Bulk operation size | 100 | `TF-LIM-0003` |
| Activity page size | 100 | `TF-QRY-0007` |

## Rate limits

Token bucket, per `(workspace, actor)`. Headers on **every** response, including
successes, so clients can slow down before being throttled
([05](05-API-SPEC.md)).

| Class | Sustained | Burst |
| --- | --- | --- |
| Reads | 1,000 / min | 100 |
| Writes | 300 / min | 50 |
| Search | 60 / min | 20 |
| Bulk | 10 / min | 3 |
| Auth (login, reset) | 10 / min **per IP and per account** | 5 |
| Invites | 50 / hour | 10 |
| Attachment uploads | 100 / hour | 10 |
| SSE connections | 10 concurrent per user | — |
| Plugin calls (per installation) | 600 / min | 100 |
| Webhook deliveries (per installation) | 1,000 / min | — |

Auth limits are per IP **and** per account: per-IP alone is defeated by a
botnet, per-account alone lets one attacker lock out every user by failing their
logins deliberately. Both, together, are required.

Service accounts get separate, higher, admin-configurable buckets so an
integration cannot exhaust a human's quota.

## Workspace quotas

Defaults; plan-dependent in a hosted deployment, configurable when self-hosted.

| Resource | Default |
| --- | --- |
| Projects | 1,000 |
| Tasks per project | 200,000 |
| Users | 10,000 |
| Teams | 500 |
| Custom roles | 100 |
| Role assignments | 50,000 |
| Saved views per user | 100 |
| Automation rules per project / workspace | 50 / 500 |
| Plugin installations | 50 |
| Webhook endpoints | 100 |
| API tokens per user | 20 |
| Attachment storage | 100 GB |
| File size | 100 MB (max 2 GB) |
| Automation runs | 10,000 / hour |

Quota exhaustion returns `TF-LIM-0004` with the resource named and the current
usage — not a generic failure. An admin should learn what to clean up from the
error itself.

## Plugin limits

From [34](34-PLUGIN-AND-EXTENSION-ARCHITECTURE.md):

| Limit | Value |
| --- | --- |
| Synchronous hook timeout | 500 ms |
| Async call timeout | 10 s |
| Webhook delivery timeout | 10 s |
| Circuit breaker threshold | 5 consecutive failures |
| Circuit open duration | 60 s |
| Delivery retries | 6, exponential backoff |
| Manifest size | 256 KB |
| Extension points per plugin | 50 |
| Plugin response body | 1 MB |

## Enforcement order

Cheapest checks first, so an attacker cannot make us do expensive work to reject
them:

```
1. connection / TLS limits          (reverse proxy)
2. body size, header size           (before parse)
3. authentication                    (cheap: one indexed read)
4. rate limit                        (bucket check)
5. request parse + field validation
6. authorization                     (cached where possible)
7. query limits                      (at filter compile)
8. domain rules                      (needs data)
9. quota                             (needs a count)
```

A request that fails at step 2 never reaches the database. This ordering is why
an unauthenticated flood costs us a body-size check rather than a query.

## Configurability

- **Hosted**: limits are plan-derived and not customer-editable.
- **Self-hosted**: every limit in this document is a configuration key with the
  documented default, because a self-hoster's constraints are theirs to judge.
- **Hard floors exist regardless of configuration** — request body, JSON depth,
  filter depth, and dependency-check depth cannot be raised past values that
  would make the server exploitable. These are security bounds, not tuning knobs.

## Acceptance gates

- **Every limit has a test** that exceeds it by one and asserts the specific code.
  A limit without a test is a limit that will be quietly removed by a refactor.
- **Order test** — an oversized body from an unauthenticated client is rejected
  without a database query, asserted by query-count instrumentation.
- **Rate-limit isolation** — exhausting one workspace's bucket does not affect
  another's ([32](32-TENANCY-AND-ISOLATION.md)).
- **Header test** — `RateLimit-*` headers are present and accurate on successful
  responses, not only on `429`.
- **Fuzz** — the filter grammar and the plugin manifest parser are fuzzed against
  their limits (`fuzz/`, [19](19-WORKSPACE-SCAFFOLD-DESIGN.md)).
