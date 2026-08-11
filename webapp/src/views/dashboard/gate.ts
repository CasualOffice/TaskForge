/**
 * A bound on how many reports a dashboard runs at once.
 *
 * # The failure this prevents
 *
 * `docs/38` gives reports a concurrency limit of **5 per workspace** and says a
 * dashboard left open "must not become a load generator". A dashboard is the
 * one surface that can breach both by simply rendering: nine tiles mount
 * together, nine `useQuery` hooks fire together, and nine of the most expensive
 * queries in the product arrive at the API in the same millisecond. Opening
 * Project health did exactly that against the dev stack and a run of the
 * requests came back `503` — the tiles that should have degraded politely
 * instead took out the tiles beside them.
 *
 * Bounding it in the browser rather than only at the edge matters because the
 * edge's answer to "too many reports" is a refusal, and a refusal renders as an
 * error in a tile whose number is perfectly computable. The user did nothing
 * wrong by opening a page.
 *
 * # Why a queue and not a stagger
 *
 * A fixed delay per tile is a guess about how long a report takes, and it is
 * wrong in both directions: too short under load, and pure latency added to
 * every fast tile when the server is idle. A semaphore is self-tuning — the
 * next tile starts the instant a slot frees.
 *
 * FIFO, so tiles start in mount order, which is reading order: the numbers at
 * the top of the dashboard resolve first.
 */

/**
 * Four, not five.
 *
 * `docs/38`'s limit of 5 is per *workspace*, and a person can have the same
 * dashboard open in two tabs — so spending the whole allowance from one tab
 * would make the second tab's tiles fail. Leaving one slot also keeps the
 * Reports page usable while a dashboard is loading in another tab.
 */
export const LIMIT = 4

let active = 0
const waiting: (() => void)[] = []

/** Test seam. The module holds process-wide state; suites must not inherit it. */
export function reset(): void {
  active = 0
  waiting.length = 0
}

export function inFlight(): number {
  return active
}

/**
 * Run `job` once a slot is free.
 *
 * `signal` is honoured **while queued**, not only while running: React Query
 * aborts a query when its component unmounts, and a dashboard someone navigated
 * away from must not keep six queued reports alive to fire one by one against a
 * page nobody is looking at.
 */
export async function throttled<T>(job: () => Promise<T>, signal?: AbortSignal): Promise<T> {
  if (signal?.aborted === true) throw abortError()

  if (active >= LIMIT) {
    await new Promise<void>((resolve, reject) => {
      const start = (): void => {
        signal?.removeEventListener('abort', cancel)
        resolve()
      }
      const cancel = (): void => {
        // Drop out of the queue rather than leaving a resolved-never promise
        // behind it: a slot released to an abandoned waiter is a slot lost for
        // the lifetime of the page.
        const at = waiting.indexOf(start)
        if (at !== -1) waiting.splice(at, 1)
        reject(abortError())
      }
      signal?.addEventListener('abort', cancel, { once: true })
      waiting.push(start)
    })
  }

  active += 1
  try {
    return await job()
  } finally {
    active -= 1
    // `shift` before calling, so a job that starts synchronously and finishes
    // synchronously cannot re-enter and hand the same slot out twice.
    const next = waiting.shift()
    if (next !== undefined) next()
  }
}

/**
 * The shape `fetch` throws on abort.
 *
 * React Query recognises an `AbortError` by name and settles the query as
 * cancelled rather than failed — a queued tile that was abandoned must not
 * render an error to a screen that has already gone.
 */
function abortError(): Error {
  const error = new Error('aborted')
  error.name = 'AbortError'
  return error
}
