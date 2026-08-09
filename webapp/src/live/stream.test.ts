/**
 * The frame parser, which is the part of live updates that fails silently.
 *
 * A wrong parser does not throw. It delivers a truncated `data`, the JSON parse
 * fails, `aggregateIdOf` returns `undefined`, and the task simply does not
 * refresh — which looks exactly like "the server did not send anything". These
 * are the cases that produce that symptom.
 */
import { describe, expect, it, vi } from 'vitest'

import { aggregateIdOf, readStream, type LiveEvent } from './stream'

/** Serve `chunks` as a streamed `text/event-stream` body. */
function serve(chunks: readonly string[]): void {
  const encoder = new TextEncoder()
  const body = new ReadableStream<Uint8Array>({
    start(controller) {
      for (const chunk of chunks) controller.enqueue(encoder.encode(chunk))
      controller.close()
    },
  })
  vi.stubGlobal(
    'fetch',
    vi.fn(() =>
      Promise.resolve(
        new Response(body, { status: 200, headers: { 'content-type': 'text/event-stream' } }),
      ),
    ),
  )
}

async function collect(chunks: readonly string[]): Promise<LiveEvent[]> {
  serve(chunks)
  const received: LiveEvent[] = []
  await readStream({
    workspaceId: 'w',
    projectId: 'p',
    signal: new AbortController().signal,
    onEvent: (event) => received.push(event),
  })
  vi.unstubAllGlobals()
  return received
}

describe('the frame parser', () => {
  it('reads id, event and data', async () => {
    const [event] = await collect(['id: 1\nevent: task.updated\ndata: {"id":"t1"}\n\n'])
    expect(event).toEqual({ id: '1', type: 'task.updated', data: '{"id":"t1"}' })
  })

  it('keeps a data field that spans several lines intact', async () => {
    // The rule this file exists for. A parser that split on '\n' would deliver
    // `{"id":"t1",` — valid-looking, unparseable, and silent.
    const [event] = await collect([
      'event: task.updated\ndata: {"id":"t1",\ndata: "title":"two\\nlines"}\n\n',
    ])
    expect(event?.data).toBe('{"id":"t1",\n"title":"two\\nlines"}')
    expect(aggregateIdOf(event as LiveEvent)).toBe('t1')
  })

  it('holds a frame split across two network chunks', async () => {
    // TCP does not respect frame boundaries. A reader that parsed each chunk
    // would lose the event that happened to straddle one.
    const events = await collect(['id: 7\neve', 'nt: task.created\ndata: {"id":"t7"}\n\n'])
    expect(events).toHaveLength(1)
    expect(events[0]?.id).toBe('7')
  })

  it('drops the heartbeat comment', async () => {
    // docs/05's 30 s heartbeat. Delivering it as an event would invalidate every
    // mounted query twice a minute on a completely idle board.
    const events = await collect([': keep-alive\n\ndata: {"id":"t1"}\n\n'])
    expect(events).toHaveLength(1)
  })

  it('surfaces the gap frame as its own type', async () => {
    const [event] = await collect([
      'event: stream.gap\ndata: {"reason":"outside_replay_window","action":"refetch"}\n\n',
    ])
    expect(event?.type).toBe('stream.gap')
  })

  it('returns no aggregate for a payload it does not recognise', async () => {
    // `data` is documented as opaque. A client that threw here would drop every
    // event type added after it shipped.
    const [event] = await collect(['event: something.new\ndata: not json\n\n'])
    expect(aggregateIdOf(event as LiveEvent)).toBeUndefined()
  })
})
