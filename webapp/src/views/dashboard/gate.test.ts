/**
 * The concurrency bound.
 *
 * Worth its own suite because it is invisible when it works: a dashboard with
 * the gate and a dashboard without it render identically against an idle
 * server, and differ only under the load that `docs/38` budgets against. The
 * only way that stays true is if something asserts the bound directly.
 */
import { afterEach, describe, expect, it } from 'vitest'

import { LIMIT, inFlight, reset, throttled } from './gate'

afterEach(() => {
  reset()
})

/** A job that never settles until told to, so overlap is observable. */
function pending(): { job: () => Promise<string>; finish: () => void } {
  let release = (): void => {}
  const gate = new Promise<void>((resolve) => {
    release = resolve
  })
  return { job: () => gate.then(() => 'done'), finish: () => release() }
}

describe('the report gate', () => {
  it('runs no more than the limit at once', async () => {
    const jobs = Array.from({ length: LIMIT + 4 }, () => pending())
    const running = jobs.map((entry) => throttled(entry.job))

    // A microtask turn, so everything that *can* start has started.
    await Promise.resolve()
    await Promise.resolve()

    expect(inFlight()).toBe(LIMIT)

    for (const entry of jobs) entry.finish()
    await Promise.all(running)
    expect(inFlight()).toBe(0)
  })

  it('starts a queued job as soon as a slot frees', async () => {
    const order: number[] = []
    const jobs = Array.from({ length: LIMIT + 1 }, () => pending())
    const running = jobs.map((entry, i) =>
      throttled(async () => {
        order.push(i)
        return entry.job()
      }),
    )

    await Promise.resolve()
    await Promise.resolve()
    // The last one has not started — there was no slot for it.
    expect(order).toEqual([...Array(LIMIT).keys()])

    jobs[0]?.finish()
    // Awaiting the finished job itself, rather than counting microtask turns:
    // the slot is released in its `finally`, so once it has settled the handoff
    // has happened or it never will.
    await running[0]
    await Promise.resolve()

    // FIFO: mount order is reading order, so the top of the dashboard resolves
    // first rather than whichever tile happened to win a race.
    expect(order).toEqual([...Array(LIMIT + 1).keys()])

    for (const entry of jobs) entry.finish()
    await Promise.all(running)
  })

  it('gives up its place in the queue when the tile goes away', async () => {
    // React Query aborts on unmount. A dashboard someone navigated away from
    // must not keep queued reports alive to fire one by one at a dead page.
    const jobs = Array.from({ length: LIMIT }, () => pending())
    const running = jobs.map((entry) => throttled(entry.job))

    const controller = new AbortController()
    let started = false
    const queued = throttled(() => {
      started = true
      return Promise.resolve('never')
    }, controller.signal)

    await Promise.resolve()
    controller.abort()

    await expect(queued).rejects.toMatchObject({ name: 'AbortError' })
    expect(started).toBe(false)

    for (const entry of jobs) entry.finish()
    await Promise.all(running)
  })

  it('does not lose a slot to a job that was abandoned while queued', async () => {
    // The bug this closes: releasing a slot to a waiter that has gone leaves
    // the slot permanently spent, and a dashboard that loses four of them stops
    // loading tiles altogether.
    const jobs = Array.from({ length: LIMIT }, () => pending())
    const running = jobs.map((entry) => throttled(entry.job))

    const controller = new AbortController()
    const abandoned = throttled(() => Promise.resolve('never'), controller.signal)
    const survivor = pending()
    const wanted = throttled(survivor.job)

    await Promise.resolve()
    controller.abort()
    await expect(abandoned).rejects.toMatchObject({ name: 'AbortError' })

    // Free one slot; it must go to the job still waiting, not to the ghost.
    jobs[0]?.finish()
    await running[0]
    await Promise.resolve()
    expect(inFlight()).toBe(LIMIT)

    survivor.finish()
    for (const entry of jobs) entry.finish()
    await Promise.all([...running, wanted])
    expect(inFlight()).toBe(0)
  })

  it('releases the slot when a job throws', async () => {
    // Otherwise one failing tile costs the dashboard a quarter of its capacity
    // for as long as the page is open.
    await expect(throttled(() => Promise.reject(new Error('boom')))).rejects.toThrow('boom')
    expect(inFlight()).toBe(0)
  })
})
