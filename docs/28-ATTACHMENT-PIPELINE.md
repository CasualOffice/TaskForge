# 28 — Attachment Pipeline

Files never pass through the API process's memory. Uploads stream browser →
object storage directly; the API mints permission, verifies the result, and
commits visibility.

## The invariant

> **An attachment row is invisible to every read path until `committed_at` is set.**

Enforced by a **partial index** rather than a runtime predicate
([22](22-DATABASE-SCHEMA.md)):

```sql
CREATE INDEX attachment_task_ix ON attachment (task_id)
    WHERE committed_at IS NOT NULL AND deleted_at IS NULL;
```

Uncommitted rows are not merely filtered out — they are *not in the index reads
use*. A forgotten `WHERE` clause cannot leak an unscanned file, because there is
no efficient path to one.

## The handshake

```
 1. POST /api/v1/tasks/{id}/attachments
      { filename, content_type, byte_size, checksum }
      → authorize task.attachment.create
      → validate size, declared type, per-workspace quota
      → INSERT attachment (committed_at = NULL, scan_status = 'PENDING')
      → return { attachment_id, upload_url, headers, expires_in: 900 }

 2. Browser PUTs bytes directly to object storage
      (pre-signed, 15 min, size-capped, content-type pinned by signature)

 3. POST /api/v1/attachments/{id}/commit
      → HEAD the object: exists? size matches? checksum matches?
      → sniff real content type from magic bytes
      → enqueue scan
      → 202 Accepted, scan_status = PENDING

 4. Scan worker
      → CLEAN    → set committed_at, emit attachment.committed → now visible
      → INFECTED → delete object, mark INFECTED, notify uploader, audit event
      → FAILED   → retry; after 3 attempts quarantine and notify an admin
```

The API never proxies the upload body. Commit reads one bounded prefix for magic
bytes; the scan worker reads the complete object. API memory use therefore does
not grow with the uploaded file size.

## Why pre-signed direct upload

Proxying uploads through the API means every large upload occupies a request
handler for its full duration, and a handful of slow clients can exhaust the
connection pool. It also puts file bytes in the same process as the permission
system. Direct-to-storage removes both.

The cost is that the object exists before the application has validated it —
which is exactly why step 3 verifies and step 4 scans **before** `committed_at`
is set.

## Validation

| Check | Where | Rule |
| --- | --- | --- |
| Size | pre-sign + storage policy | ≤ 100 MB default, workspace-configurable to 2 GB |
| Declared type | pre-sign | against an allow/deny list |
| **Real type** | commit | magic-byte sniff; mismatch with the declared type ⇒ reject |
| Checksum | commit | client-supplied SHA-256 must match |
| Quota | pre-sign | per-workspace storage total |
| Malware | scan worker | ClamAV by default; pluggable |

**Content type is never trusted from the client.** The declared type is used only
to pin the pre-signed policy; the *stored* type comes from magic bytes at commit.
A file uploaded as `image/png` that is actually HTML is rejected — that mismatch
is the stored-XSS vector.

### Commit transaction boundary

Object-store I/O never runs inside a database transaction. Commit first reads
and authorizes the pending row in a short transaction, closes it, then performs
HEAD and bounded-prefix reads. A second short transaction re-authorizes the
caller and claims `verified_at` while recording the sniffed type, activity,
audit and outbox event.

`verified_at` is a compare-and-set marker, not visibility. Two concurrent
commit requests may inspect the same bytes, but only one can change
`verified_at` from `NULL`; only that winner writes the event that wakes the scan
consumer. Visibility still changes only when a clean verdict sets
`committed_at`. The cost is a second authority resolution on commit, paid on a
rare write to keep object latency outside locks and connections.

## Serving downloads

```
GET /api/v1/attachments/{id}/download
  → authorize task.attachment.read
  → 302 to a pre-signed GET URL, 5 min, single-use where the backend supports it
```

- Object keys are `{workspace_id}/{task_id}/{attachment_id}` and are never
  guessable ([32](32-TENANCY-AND-ISOLATION.md)); a URL alone is not authority.
- Responses carry `Content-Disposition: attachment` and
  `X-Content-Type-Options: nosniff`.
- User content is served from a **separate origin** from the application. This is
  the single most important control here: it means a stored HTML or SVG file
  cannot execute in the application's origin even if every other check fails.
- Inline preview (images, PDF) renders in a sandboxed viewer on that separate
  origin.
- Downloads are audited when the workspace enables download auditing.

## Orphan and lifecycle cleanup

| Case | Handling |
| --- | --- |
| Pre-signed but never uploaded | row swept after 24 h |
| Uploaded but never committed | object + row swept after 24 h |
| Infected | object deleted immediately; row retained with status for the audit trail |
| Attachment deleted | soft delete; object removed after the 30-day grace |
| Task hard-deleted | objects removed with the task |
| Workspace hard-deleted | the entire `{workspace_id}/` prefix removed |

The sweeper reconciles in both directions — rows without objects *and* objects
without rows. One-directional cleanup leaves storage that nothing references and
nobody is billed correctly for.

## Local deployment

The single-node profile ([48](48-DEPLOYMENT-PROFILES.md)) has no S3. A filesystem
backend implements the same trait, with the API issuing short-lived signed local
URLs served by a dedicated handler that streams with bounded buffers.

**The pipeline is identical** — same states, same handshake, same scan step. A
deployment profile must not change the security model, or the small profile
becomes the insecure one.

## Limits

| Limit | Value |
| --- | --- |
| File size | 100 MB default, max 2 GB |
| Files per task | 100 |
| Workspace storage | plan-dependent |
| Pre-signed upload TTL | 15 min |
| Download URL TTL | 5 min |
| Concurrent uploads per user | 5 |
| Scan timeout | 60 s |

## Acceptance gates

- **Streaming test** — uploading a 2 GB file leaves API process RSS flat.
- **Invisibility test** — an uncommitted attachment is absent from every read
  path, including search, the API, exports, and SSE.
- **Type-confusion test** — HTML uploaded as `image/png` is rejected at commit.
- **Infected-file test** — an EICAR test file is quarantined, the object deleted,
  the uploader notified, and an audit event written.
- **Orphan test** — abandoned uploads are swept in both directions within 24 h.
- **Cross-tenant test** — a pre-signed URL for workspace A cannot be minted or
  used by a member of workspace B.
- **Separate-origin test** — a stored HTML attachment cannot access application
  cookies or storage.
