/**
 * Attachments — `docs/28`'s three-step pipeline, from the browser.
 *
 * # Why the bytes do not go through the API
 *
 * `POST /tasks/{id}/attachments` returns a **presigned URL** and the client
 * `PUT`s to it directly, then calls `commit`. The API process never holds the
 * file: a 2 GiB upload through the application would be 2 GiB of a request
 * handler's memory, and the same discipline governs downloads and exports.
 *
 * # Why this module did not exist until now
 *
 * The server has had the whole pipeline for a long time — presign, the object
 * origin, commit, scan, download — and the client had none of it, because the
 * upload was unreachable from a browser. The attachment origin is deliberately
 * a *different* origin (`docs/28` §Serving downloads: a stored HTML file must
 * not execute in the application's), and a cross-origin `PUT` carrying
 * `Content-Type` is not a simple request, so the browser sends `OPTIONS` first
 * — which that router had no route for and answered `405`. Presign worked, the
 * `PUT` never left the browser, and the product looked as though attachments
 * were unimplemented. The preflight is served now.
 *
 * # The checksum is the client's promise, not the server's trust
 *
 * `presign` takes a SHA-256 of the bytes about to be uploaded and `commit`
 * re-derives everything from what actually landed — size, type, digest. The
 * value here is what lets the server *refuse* a mismatch; it is not evidence of
 * anything on its own.
 */
import { request } from './http'

/** `attachments::wire::AttachmentBody`. */
export interface Attachment {
  readonly id: string
  readonly filename: string
  readonly content_type: string
  readonly byte_size: number
  readonly status: string
  readonly created_at: string
  readonly created_by: string
}

interface Presigned {
  readonly attachment_id: string
  readonly upload_url: string
  /** Headers the `PUT` must carry. The signature pins the method and the key. */
  readonly headers: readonly (readonly [string, string])[]
  readonly expires_in: number
}

export function listAttachments(
  workspaceId: string,
  taskId: string,
  signal?: AbortSignal,
): Promise<{ data: readonly Attachment[] }> {
  return request<{ data: readonly Attachment[] }>(`/api/v1/tasks/${taskId}/attachments`, {
    workspaceId,
    signal,
  })
}

/**
 * Lower-case hex SHA-256, which is the form `presign` validates.
 *
 * `crypto.subtle` needs a secure context; every deployment of this app is one,
 * because the session cookie is `Secure` and an insecure origin could not
 * authenticate at all. Same argument `idempotencyKey` makes for `randomUUID`.
 */
export async function checksumOf(file: File): Promise<string> {
  const digest = await crypto.subtle.digest('SHA-256', await file.arrayBuffer())
  return [...new Uint8Array(digest)].map((byte) => byte.toString(16).padStart(2, '0')).join('')
}

/**
 * Upload one file and return the attachment once it is committed.
 *
 * Three calls, in `docs/28`'s order, and the failure modes are not the same:
 *
 * - **presign** refused: nothing happened. The permission, the size or the type
 *   was rejected before any bytes moved.
 * - **PUT** failed: an attachment row exists in `PENDING` and no object backs
 *   it. The sweeper collects those; the caller sees an error and may retry,
 *   which mints a *new* row rather than reusing a half-finished one.
 * - **commit** refused: the bytes are there and the server has looked at them
 *   and declined — a checksum mismatch, a size disagreement, a type the magic
 *   bytes contradict. That is the interesting refusal, and it is the one worth
 *   showing verbatim.
 */
export async function uploadAttachment(
  workspaceId: string,
  taskId: string,
  file: File,
): Promise<Attachment> {
  const presigned = await request<Presigned>(`/api/v1/tasks/${taskId}/attachments`, {
    method: 'POST',
    workspaceId,
    body: {
      filename: file.name,
      // Browsers leave `type` empty for extensions they do not recognise, and
      // the server requires one. `application/octet-stream` is the honest
      // answer for "bytes of an unknown kind" and commit re-derives the truth
      // from the magic bytes anyway.
      content_type: file.type === '' ? 'application/octet-stream' : file.type,
      byte_size: file.size,
      checksum: await checksumOf(file),
    },
  })

  // Not through `request`: this goes to the attachment origin, carries no
  // session and no CSRF token, and must not — the presigned signature is the
  // authority, and sending a cookie to another origin is what that separation
  // exists to prevent.
  const put = await fetch(presigned.upload_url, {
    method: 'PUT',
    headers: Object.fromEntries(presigned.headers.map(([name, value]) => [name, value])),
    body: file,
  })
  if (!put.ok) throw new Error(`the upload was refused (${put.status})`)

  return request<Attachment>(`/api/v1/attachments/${presigned.attachment_id}/commit`, {
    method: 'POST',
    workspaceId,
  })
}

/**
 * Where to send someone to get the file.
 *
 * A redirect to a short-lived signed URL on the attachment origin, which is why
 * this is a plain link and not a fetch: following it is the browser's job, and
 * reading the bytes into this process to hand them back would undo the reason
 * the pipeline is shaped this way.
 */
export function downloadUrl(attachmentId: string): string {
  return `/api/v1/attachments/${attachmentId}/download`
}
