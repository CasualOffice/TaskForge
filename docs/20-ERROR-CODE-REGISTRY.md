# 20 — Error-Code Registry

Stable, namespaced codes so clients can react programmatically, logs are
greppable, and support can be answered from a code rather than a screenshot.

Codes are **append-only**: never reused, never repurposed, retired with a note. A
code is part of the public contract once shipped.

## Format

`TF-<AREA>-<NNNN>` — `TF` = TaskForge.

| Area | Subsystem |
| --- | --- |
| `AUT` | Authentication / session ([40](40-IDENTITY-AUTH-AND-SESSION.md)) |
| `AZN` | Authorization ([04](04-RBAC-AND-AUTHORIZATION.md)) |
| `VAL` | Request validation |
| `QRY` | Filter / sort / pagination ([27](27-FILTER-AND-SAVED-VIEW-DSL.md)) |
| `WFL` | Workflow / transitions ([23](23-WORKFLOW-AND-STATE-MACHINE.md)) |
| `TSK` | Task domain rules ([03](03-DOMAIN-MODEL.md)) |
| `PRJ` | Project / workspace |
| `CNC` | Concurrency ([24](24-CONCURRENCY-AND-IDEMPOTENCY.md)) |
| `IDM` | Idempotency |
| `ATT` | Attachments ([28](28-ATTACHMENT-PIPELINE.md)) |
| `PLG` | Plugins ([34](34-PLUGIN-AND-EXTENSION-ARCHITECTURE.md)) |
| `AUM` | Automation ([36](36-AUTOMATION-RULES-DESIGN.md)) |
| `LIM` | Rate limits / quotas ([21](21-API-LIMITS-AND-QUOTAS.md)) |
| `SYS` | Internal / infrastructure |

## Registry

### Authentication — `AUT`

| Code | Meaning | HTTP |
| --- | --- | --- |
| `TF-AUT-0001` | Not authenticated | 401 |
| `TF-AUT-0002` | Session expired | 401 |
| `TF-AUT-0003` | Session revoked | 401 |
| `TF-AUT-0004` | Invalid credentials | 401 |
| `TF-AUT-0005` | MFA required | 401 |
| `TF-AUT-0006` | MFA code invalid | 401 |
| `TF-AUT-0007` | Re-authentication required for this action | 401 |
| `TF-AUT-0008` | CSRF token missing or invalid | 403 |
| `TF-AUT-0009` | Token expired | 401 |
| `TF-AUT-0010` | Token revoked | 401 |
| `TF-AUT-0011` | SSO required for this workspace | 403 |
| `TF-AUT-0012` | Account locked — too many attempts | 429 |
| `TF-AUT-0013` | Credential type not permitted for this endpoint | 403 |
| `TF-AUT-0014` | MFA already enrolled for this account | 409 |

`TF-AUT-0013` is not `TF-AZN-0001`. It is a fact about the *credential*, not
about a grant: a workspace-scoped bearer token cannot create a different
workspace no matter what its principal holds, and the fix is a different
credential rather than a different role.

### Authorization — `AZN`

| Code | Meaning | HTTP |
| --- | --- | --- |
| `TF-AZN-0001` | Permission denied (no grant) | 403 |
| `TF-AZN-0002` | Permission denied (constraint unsatisfied) | 403 |
| `TF-AZN-0003` | Grant ceiling exceeded — cannot grant what you do not hold | 422 |
| `TF-AZN-0004` | Scope ceiling exceeded for this assignment | 422 |
| `TF-AZN-0005` | Cannot remove the last workspace owner | 422 |
| `TF-AZN-0006` | Self-elevation rejected | 422 |
| `TF-AZN-0007` | Not a member of the target workspace | 403 |
| `TF-AZN-0008` | Not found, or not visible to you | 404 |

`TF-AZN-0008` is the generic form of `TF-PRJ-0001` and `TF-TSK-0001`, for the
resources that have no code of their own. It sits in `AZN` and not in `VAL`
because it is a **visibility** answer, not a validation one: `docs/04` requires
absent and invisible to be indistinguishable, so one code has to cover both and
the body must never say which. Anything that can be seen and may not be touched
is `TF-AZN-0001` or `-0002` instead.

`TF-AZN-0001` and `-0002` are distinct on purpose: the first means "you were never
given this," the second means "you have it, but not for this object." They lead a
user to different actions, and `/permissions/explain` returns the difference.

### Validation — `VAL`

| Code | Meaning | HTTP |
| --- | --- | --- |
| `TF-VAL-0001` | Malformed request body | 400 |
| `TF-VAL-0002` | Unknown field in request | 400 |
| `TF-VAL-0003` | Required field missing | 400 |
| `TF-VAL-0004` | Field value out of range | 400 |
| `TF-VAL-0005` | Invalid enum value | 400 |
| `TF-VAL-0006` | Invalid ID format | 400 |
| `TF-VAL-0007` | Referenced entity not found | 422 |
| `TF-VAL-0008` | Referenced entity belongs to another project | 422 |

### Query — `QRY`

| Code | Meaning | HTTP |
| --- | --- | --- |
| `TF-QRY-0001` | Unknown filter field | 400 |
| `TF-QRY-0002` | Unknown or unsortable sort field | 400 |
| `TF-QRY-0003` | Operator not valid for this field type | 400 |
| `TF-QRY-0004` | Too many filter clauses | 400 |
| `TF-QRY-0005` | Filter nesting too deep | 400 |
| `TF-QRY-0006` | Invalid or expired cursor | 400 |
| `TF-QRY-0007` | Page size over limit | 400 |
| `TF-QRY-0008` | Search query too long | 400 |
| `TF-QRY-0009` | Query timed out | 503 |

### Workflow — `WFL`

| Code | Meaning | HTTP |
| --- | --- | --- |
| `TF-WFL-0001` | Status cannot be set directly — use a transition | 400 |
| `TF-WFL-0002` | No such transition in this workflow | 422 |
| `TF-WFL-0003` | Transition requires a permission you lack | 403 |
| `TF-WFL-0004` | Required fields missing for the target status | 422 |
| `TF-WFL-0005` | Blocking dependencies unresolved | 422 |
| `TF-WFL-0006` | Cannot delete a status holding tasks — supply `migrate_to` | 422 |
| `TF-WFL-0007` | Workflow must have exactly one initial status | 422 |
| `TF-WFL-0008` | Status belongs to a different workflow | 422 |
| `TF-WFL-0009` | Status name already in use in this workflow | 409 |
| `TF-WFL-0010` | That transition already exists | 409 |
| `TF-WFL-0011` | Too many tasks to migrate in a request — run it as a job | 422 |

### Task — `TSK`

| Code | Meaning | HTTP |
| --- | --- | --- |
| `TF-TSK-0001` | Task not found or not visible | 404 |
| `TF-TSK-0002` | Task is deleted | 410 |
| `TF-TSK-0003` | Dependency would create a cycle | 422 |
| `TF-TSK-0004` | Subtask nesting limit exceeded | 422 |
| `TF-TSK-0005` | Assignee is not a member of the project | 422 |
| `TF-TSK-0006` | Parent task must be in the same project | 422 |
| `TF-TSK-0007` | Task limit for this project reached | 422 |

### Project — `PRJ`

| Code | Meaning | HTTP |
| --- | --- | --- |
| `TF-PRJ-0001` | Project not found or not visible | 404 |
| `TF-PRJ-0002` | Project key already in use | 409 |
| `TF-PRJ-0003` | Project key is immutable | 422 |
| `TF-PRJ-0004` | Project key format invalid | 400 |
| `TF-PRJ-0005` | Cannot delete an environment in use — supply a migration target | 422 |
| `TF-PRJ-0006` | Cannot remove the last member of a workspace | 422 |
| `TF-PRJ-0007` | Workspace slug already in use | 409 |
| `TF-PRJ-0008` | Team name already in use in this workspace | 409 |
| `TF-PRJ-0009` | Environment name already in use in this project | 409 |
| `TF-PRJ-0010` | Milestone name already in use in this project | 409 |
| `TF-PRJ-0011` | Tag name already in use at that scope | 409 |
| `TF-PRJ-0012` | Milestone limit for this project reached | 422 |
| `TF-PRJ-0013` | Tag limit for this workspace reached | 422 |
| `TF-PRJ-0014` | Role name already in use in this workspace | 409 |
| `TF-PRJ-0015` | Release name already in use in this project | 409 |
| `TF-PRJ-0016` | Release not found | 404 |

`TF-PRJ-0011` says "at that scope" and not "in this workspace" because a tag is
`TF-PRJ-0013` bounds the tag vocabulary at the door. A tag is a user-authored

`TF-WFL-0011` exists because `docs/23` puts a ceiling on the synchronous path:
a status delete moves every task on it in one transaction, and above 10,000
that is a tracked background job with progress rather than a request. The code
says which side of that line the caller is on, so a client can offer the job
instead of retrying a request that will never fit.

`TF-PRJ-0006` is not `TF-AZN-0005`. That one protects the last *owner* — a
grant — and this one protects the last *member*, which is a different fact: a
workspace with no members is unreachable forever, because nothing can see it and
therefore nothing can add a member back to it. Both survive; neither implies the
other.

### Concurrency & idempotency — `CNC`, `IDM`

| Code | Meaning | HTTP |
| --- | --- | --- |
| `TF-CNC-0001` | Version conflict | 409 |
| `TF-CNC-0002` | `If-Match` required | 428 |
| `TF-CNC-0003` | Malformed `If-Match` | 400 |
| `TF-CNC-0004` | Export not ready for download | 409 |
| `TF-IDM-0001` | Request with this idempotency key is in progress | 409 |
| `TF-IDM-0002` | Idempotency key reused with a different body | 422 |
| `TF-IDM-0003` | Idempotency key required | 400 |

### Attachments — `ATT`

| Code | Meaning | HTTP |
| --- | --- | --- |
| `TF-ATT-0001` | File exceeds size limit | 413 |
| `TF-ATT-0002` | Content type not permitted | 415 |
| `TF-ATT-0003` | Declared type does not match content | 422 |
| `TF-ATT-0004` | Checksum mismatch | 422 |
| `TF-ATT-0005` | Upload not found or expired | 404 |
| `TF-ATT-0006` | Malware detected | 422 |
| `TF-ATT-0007` | Scan pending — not yet available | 409 |
| `TF-ATT-0008` | Workspace storage quota exceeded | 507 |
| `TF-ATT-0009` | Uploaded object does not match the declared size | 422 |
| `TF-ATT-0010` | Scan did not complete; the file will not be served | 422 |
| `TF-ATT-0011` | Task attachment limit reached | 422 |

`TF-ATT-0009` is not `TF-ATT-0001`: "larger than you are allowed" is a rule the
caller can fix by uploading something smaller, and "the object is not the size
you told us it would be" means the upload and the declaration disagree — which
is a broken client or a tampered upload. `TF-ATT-0010` is not `TF-ATT-0007`
either: pending is a wait, and failed is a refusal (**D-061**).

### Plugins — `PLG`

| Code | Meaning | HTTP |
| --- | --- | --- |
| `TF-PLG-0001` | Blocked by a plugin validation hook | 422 |
| `TF-PLG-0002` | Invalid plugin manifest | 400 |
| `TF-PLG-0003` | Plugin contract version incompatible | 422 |
| `TF-PLG-0004` | Requested scope not consented | 403 |
| `TF-PLG-0005` | Plugin scope exceeds installer's permissions | 422 |
| `TF-PLG-0006` | Plugin timed out | 504 |
| `TF-PLG-0007` | Plugin circuit breaker open | 503 |
| `TF-PLG-0008` | Plugin quota exceeded | 429 |
| `TF-PLG-0009` | Signature verification failed | 401 |
| `TF-PLG-0010` | Egress destination not allow-listed | 403 |

### Automation — `AUM`

| Code | Meaning | HTTP |
| --- | --- | --- |
| `TF-AUM-0001` | Automation depth limit exceeded | — (logged) |
| `TF-AUM-0002` | Rule limit reached | 422 |
| `TF-AUM-0003` | `run_as` principal lacks required permission | 422 |
| `TF-AUM-0004` | `run_as` exceeds author's permissions | 422 |
| `TF-AUM-0005` | Rule disabled after repeated failures | — (notified) |
| `TF-AUM-0006` | Per-task automation rate exceeded | — (logged) |

### Limits — `LIM`

| Code | Meaning | HTTP |
| --- | --- | --- |
| `TF-LIM-0001` | Rate limit exceeded | 429 |
| `TF-LIM-0002` | Concurrent request limit exceeded | 429 |
| `TF-LIM-0003` | Bulk operation size exceeded | 400 |
| `TF-LIM-0004` | Workspace quota exceeded | 422 |
| `TF-LIM-0005` | Request body too large | 413 |

### System — `SYS`

| Code | Meaning | HTTP |
| --- | --- | --- |
| `TF-SYS-0001` | Internal error | 500 |
| `TF-SYS-0002` | Service temporarily unavailable | 503 |
| `TF-SYS-0003` | Database unavailable | 503 |
| `TF-SYS-0004` | Dependency unavailable | 503 |
| `TF-SYS-0005` | Request timed out | 504 |
| `TF-SYS-0006` | Load shedding — retry later | 503 |
| `TF-SYS-0007` | Designed but not built in this deployment | 501 |

## Rules

1. Every error response carries a code, a human message, a `request_id`, and a
   docs URL ([05](05-API-SPEC.md)).
2. `details` returns **every** violation, not the first — a form must not reveal
   its requirements one round-trip at a time.
3. **Messages never leak cross-tenant information.** Invisible resources are
   `404` with no distinction from absent ones ([32](32-TENANCY-AND-ISOLATION.md)).
4. `TF-SYS-0001` never includes a stack trace, a SQL fragment, or an internal
   hostname. The `request_id` is how an operator correlates it to the log that
   does.
5. Adding a code is a documentation change (this registry) plus the tracker row
   that introduced it.
6. Retiring a code leaves the row with a `Retired in vN` note. The number is
   never reused.
