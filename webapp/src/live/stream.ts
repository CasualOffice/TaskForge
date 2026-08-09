/**
 * Reading `GET /api/v1/stream` — server-sent events, over `fetch`.
 *
 * # Why not `EventSource`
 *
 * `EventSource` cannot set request headers. The stream is workspace-scoped and
 * `/api/v1/stream` has no `{workspace_id}` path segment, so the tenant can only
 * arrive in `X-Workspace-Id` — which `EventSource` has no way to send, and
 * `WorkspaceMember` answers 404 without. The same limitation would cost the
 * reconnect contract: `docs/05` resumes a gap with `Last-Event-ID`, and
 * `EventSource` only sends that header on *its own* automatic reconnects, which
 * it does with its own timing and no way to stop.
 *
 * So the frames are parsed here. It is about sixty lines, and it buys the two
 * things the built-in cannot do.
 *
 * # The parser's one rule
 *
 * A frame ends at a blank line, and a `data:` field may span several lines. A
 * reader that split the body on newlines and treated each as a frame works
 * perfectly against every small test payload and corrupts the first event whose
 * JSON contains a newline. The buffer below therefore splits on `\n\n` and
 * nothing else.
 */

/** One decoded frame. `id` is what a reconnect sends back as `Last-Event-ID`. */
export interface LiveEvent {
  readonly id: string | undefined
  /** `task.created`, `task.updated`, `stream.gap`, … */
  readonly type: string
  readonly data: string
}

export interface StreamOptions {
  readonly workspaceId: string
  readonly projectId: string
  /** Resume point from a previous connection, if there is one. */
  readonly lastEventId?: string | undefined
  readonly signal: AbortSignal
  readonly onEvent: (event: LiveEvent) => void
}

/**
 * Hold a stream open until it ends or is aborted.
 *
 * Resolves when the server closes it — `docs/05` and D-041 make that a *normal*
 * outcome, not an error: `SIGTERM` closes every live stream deliberately so a
 * client sees end-of-stream and reconnects, rather than a socket vanishing
 * mid-frame. Rejects only when the connection could not be established.
 */
export async function readStream(options: StreamOptions): Promise<void> {
  const response = await fetch(
    `/api/v1/stream?project_id=${encodeURIComponent(options.projectId)}`,
    {
      headers: {
        accept: 'text/event-stream',
        'x-workspace-id': options.workspaceId,
        ...(options.lastEventId === undefined ? {} : { 'last-event-id': options.lastEventId }),
      },
      credentials: 'include',
      signal: options.signal,
    },
  )

  if (!response.ok || response.body === null) {
    throw new Error(`stream refused: ${response.status}`)
  }

  const reader = response.body.getReader()
  const decoder = new TextDecoder()
  let buffer = ''

  for (;;) {
    const { done, value } = await reader.read()
    if (done) return
    // `stream: true` so a multi-byte character split across two network chunks
    // is held rather than decoded into a replacement character — which would
    // corrupt exactly the non-English task titles nobody tests with.
    buffer += decoder.decode(value, { stream: true })

    let boundary = buffer.indexOf('\n\n')
    while (boundary !== -1) {
      const frame = buffer.slice(0, boundary)
      buffer = buffer.slice(boundary + 2)
      const event = parseFrame(frame)
      if (event !== undefined) options.onEvent(event)
      boundary = buffer.indexOf('\n\n')
    }
  }
}

/** One frame's fields, or `undefined` for a heartbeat comment. */
function parseFrame(frame: string): LiveEvent | undefined {
  let id: string | undefined
  let type = 'message'
  const data: string[] = []

  for (const line of frame.split('\n')) {
    // A line beginning with `:` is a comment — which is exactly what the 30 s
    // heartbeat is. Dropping it here is why an idle stream does not deliver a
    // stream of empty events.
    if (line === '' || line.startsWith(':')) continue
    const colon = line.indexOf(':')
    const field = colon === -1 ? line : line.slice(0, colon)
    // One optional space after the colon is part of the framing, not the value.
    const rest = colon === -1 ? '' : line.slice(colon + 1).replace(/^ /, '')
    if (field === 'id') id = rest
    else if (field === 'event') type = rest
    else if (field === 'data') data.push(rest)
  }

  if (data.length === 0 && id === undefined) return undefined
  return { id, type, data: data.join('\n') }
}

/**
 * The aggregate an event is about, if its payload names one.
 *
 * Tolerant on purpose: the `data` field is documented as opaque JSON, and a
 * client that threw on an unfamiliar shape would drop every event added after it
 * shipped. An unparseable payload invalidates by prefix instead, which is slower
 * and never wrong.
 */
export function aggregateIdOf(event: LiveEvent): string | undefined {
  try {
    const parsed: unknown = JSON.parse(event.data)
    if (typeof parsed === 'object' && parsed !== null && 'id' in parsed) {
      const id = (parsed as { id: unknown }).id
      return typeof id === 'string' ? id : undefined
    }
  } catch {
    // Not JSON, or not a shape this build knows. See the doc comment.
  }
  return undefined
}
