/**
 * The attachments panel.
 *
 * # Why this exists now and did not before
 *
 * `docs/28`'s pipeline has been complete on the server for a long time —
 * presign, a separate object origin, commit, scan, download — and the client
 * had none of it. The reason is worth keeping: the attachment origin is
 * *deliberately* a different origin, so a browser `PUT` carrying a content type
 * is a preflighted request, and that router had no `OPTIONS` route. Presign
 * returned a URL the browser then refused to use. `task/unbuilt.ts` said
 * attachments were unavailable, and it was right until the preflight was
 * served.
 *
 * # Why an upload can finish and still not be downloadable
 *
 * `commit` answers `202`, not `200`: the malware scan has not run yet, and
 * `docs/28` fails closed — a deployment with no scanner leaves the row
 * `PENDING` and it is never downloadable (D-062, countersigned). So the row
 * shows its status rather than pretending every upload ends in a file: a
 * download link that 409s is worse than a line saying the scan has not
 * finished.
 */
import { useRef, useState, type ReactElement } from 'react'
import { Button } from '@schnsrw/design-system'
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query'

import { downloadUrl, listAttachments, uploadAttachment } from '../api/attachments'
import { keys } from '../api/keys'
import { ErrorNotice } from '../shell/notice'
import { useAnnounce } from '../shell/announce'
import { useWorkspaceId } from '../shell/session'

/** Bytes, as a person would say them. */
export function fileSize(bytes: number): string {
  if (bytes < 1024) return `${bytes} B`
  const kib = bytes / 1024
  if (kib < 1024) return `${Math.round(kib)} KB`
  const mib = kib / 1024
  if (mib < 1024) return `${mib.toFixed(1)} MB`
  return `${(mib / 1024).toFixed(1)} GB`
}

/**
 * What a status means to someone looking at it.
 *
 * The enum's own words are a state machine's vocabulary. "Scanning" is what a
 * person needs to know, and `INFECTED` needs a sentence, not a label.
 */
const STATUS_COPY: Record<string, string> = {
  PENDING: 'Waiting for a scan',
  SCANNING: 'Scanning',
  CLEAN: '',
  INFECTED: 'Blocked — malware detected',
  FAILED: 'Upload did not finish',
}

export function Attachments({
  taskId,
  mayUpload,
}: {
  taskId: string
  mayUpload: boolean
}): ReactElement | null {
  const workspaceId = useWorkspaceId()
  const client = useQueryClient()
  const announce = useAnnounce()
  const picker = useRef<HTMLInputElement>(null)
  const [rejected, setRejected] = useState<string | undefined>(undefined)
  // How many files this session uploaded. Kept because the list will not show
  // them: `GET /tasks/{id}/attachments` returns only committed rows, and
  // `committed_at` is set by the `PENDING → CLEAN` transition alone. Without
  // that note an upload that worked perfectly looks like one that vanished.
  const [uploaded, setUploaded] = useState(0)

  const attachments = useQuery({
    queryKey: keys.attachments(workspaceId, taskId),
    queryFn: ({ signal }) => listAttachments(workspaceId, taskId, signal),
    enabled: workspaceId !== '',
  })

  const upload = useMutation({
    mutationFn: (files: readonly File[]) =>
      // Sequential, not `Promise.all`: each file is a presign, a cross-origin
      // PUT and a commit, and firing ten at once is thirty requests racing for
      // the same rate limit. The list also updates in the order someone chose.
      files.reduce<Promise<unknown>>(
        (previous, file) => previous.then(() => uploadAttachment(workspaceId, taskId, file)),
        Promise.resolve(),
      ),
    onSuccess: (_result, files) => {
      announce(`Uploaded ${files.length === 1 ? '1 file' : `${files.length} files`}.`)
      setUploaded(files.length)
      void client.invalidateQueries({ queryKey: keys.attachments(workspaceId, taskId) })
    },
  })

  const rows = attachments.data?.data ?? []

  return (
    <section className="card attach" aria-labelledby="attach-title">
      <header className="attach__head">
        <h2 className="card__title" id="attach-title">
          Attachments
        </h2>
        {mayUpload ? (
          <>
            {/* A real file input, hidden, driven by a button. The native
                control cannot be styled to match anything else on the page, and
                replacing it with a `<div>` would lose keyboard operation and
                the drop target the browser gives for free. */}
            <input
              ref={picker}
              type="file"
              multiple
              className="visually-hidden"
              onChange={(event) => {
                const files = [...(event.target.files ?? [])]
                setRejected(undefined)
                if (files.length > 0) upload.mutate(files)
                // Cleared so choosing the same file twice fires again.
                event.target.value = ''
              }}
            />
            <Button
              size="sm"
              disabled={upload.isPending}
              onClick={() => picker.current?.click()}
            >
              {upload.isPending ? 'Uploading…' : 'Add files'}
            </Button>
          </>
        ) : null}
      </header>

      {attachments.error ? <ErrorNotice error={attachments.error} /> : null}
      {upload.error ? <ErrorNotice error={upload.error} /> : null}
      {rejected === undefined ? null : <p className="field__hint">{rejected}</p>}

      {uploaded === 0 ? null : (
        // The reader's language, not the repository's: a document number in
        // product copy tells someone nothing they can act on.
        <p className="attach__pending" role="status">
          {uploaded === 1 ? '1 file uploaded' : `${uploaded} files uploaded`}, waiting to be
          checked for viruses. Files appear here once that check passes.
        </p>
      )}

      {attachments.isPending ? (
        <p className="empty">Loading attachments…</p>
      ) : rows.length === 0 ? (
        <p className="empty">Nothing attached.</p>
      ) : (
        <ul className="attach__list">
          {rows.map((file) => {
            const note = STATUS_COPY[file.status] ?? file.status
            const downloadable = file.status === 'CLEAN'
            return (
              <li className="attach__row" key={file.id}>
                <span className="attach__name">
                  {downloadable ? (
                    // A plain link: following it is the browser's job, and the
                    // endpoint redirects to a short-lived signed URL on the
                    // attachment origin.
                    <a href={downloadUrl(file.id)} download>
                      {file.filename}
                    </a>
                  ) : (
                    file.filename
                  )}
                </span>
                <span className="attach__meta">{fileSize(file.byte_size)}</span>
                {note === '' ? null : <span className="attach__status">{note}</span>}
              </li>
            )
          })}
        </ul>
      )}
    </section>
  )
}
