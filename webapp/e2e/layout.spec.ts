/**
 * Geometry, in a real browser (`docs/47` §9).
 *
 * Every assertion here is about something jsdom cannot see: whether an element
 * is on the screen, how wide it is, what order it is in, and how big it is under
 * a finger. Each one corresponds to a defect that shipped — all of them passed
 * `tsc`, `eslint` and the jsdom suite on the way out.
 */
import { expect, test } from '@playwright/test'

import { stubApi } from './stub'

test.beforeEach(async ({ page }) => {
  await stubApi(page)
})

/** Nothing may push the page sideways. `docs/47` §5: no horizontal scrolling. */
for (const path of ['/', '/board', '/settings/profile', '/settings/roles', '/reports']) {
  test(`${path} never scrolls sideways`, async ({ page, isMobile }) => {
    // Audit item 2, still open: the shell is wider than a 390 px viewport on
    // several routes. `test.fail` rather than `skip` — the assertion runs, and
    // the day someone fixes the layout this reports an *unexpected pass* and
    // makes them delete this line. A skipped test is a test nobody removes.
    test.fail(isMobile, 'audit item 2 — the narrow shell still overflows')
    await page.goto(path)
    await page.waitForLoadState('networkidle')
    const overflow = await page.evaluate(() => ({
      scrollWidth: document.documentElement.scrollWidth,
      clientWidth: document.documentElement.clientWidth,
    }))
    // One pixel of tolerance for sub-pixel rounding, and no more: two is a
    // layout that is actually wider than the screen.
    expect(overflow.scrollWidth).toBeLessThanOrEqual(overflow.clientWidth + 1)
  })
}

test('the task list keeps its title readable', async ({ page, isMobile }) => {
  // Audit item 3 on a phone. The stacked row landed in #79 but is unverified
  // there — this is what verifies it, and it does not pass yet.
  test.fail(isMobile, 'audit item 3 — the stacked row does not hold at 390 px')
  // The reported failure: 492 px of fixed columns before the title's `1fr`, so
  // on a phone the fixed columns ate the row and the title — the one thing
  // anyone scans for — was the column that collapsed.
  await page.goto('/')
  const title = page.locator('.list__cell--title').first()
  await expect(title).toBeVisible()

  // The invariant is not a fraction of the screen — a sidebar and five other
  // columns make that meaningless — it is that the title is the **widest**
  // thing in its row. Every other column is a detail; the title is the row, and
  // it is the one that must never be what shrinks.
  const widths = await page.evaluate(() => {
    const row = document.querySelector('.list__row')
    if (row === null) return null
    return [...row.querySelectorAll('.list__cell')].map((cell) => ({
      title: cell.classList.contains('list__cell--title'),
      width: Math.round(cell.getBoundingClientRect().width),
    }))
  })
  expect(widths).not.toBeNull()
  const titleWidth = widths!.find((cell) => cell.title)?.width ?? 0
  const widest = Math.max(...widths!.filter((cell) => !cell.title).map((cell) => cell.width))
  expect(titleWidth, `title ${titleWidth}px vs widest other column ${widest}px`).toBeGreaterThan(
    widest,
  )
  // And wide enough to read a title in, not merely wider than a date.
  expect(titleWidth).toBeGreaterThan(200)
})

test('the create form opens fully on screen', async ({ page }) => {
  // The reported failure: a 360 px panel right-aligned to a trigger near the
  // left edge extends off the screen, and an absolutely positioned surface is
  // clipped by any ancestor that hides overflow.
  await page.goto('/')
  await page.getByRole('button', { name: 'New task' }).click()
  const surface = page.locator('.pop__surface')
  await expect(surface).toBeVisible()
  const box = await surface.boundingBox()
  const viewport = page.viewportSize()
  expect(box).not.toBeNull()
  expect(box!.x).toBeGreaterThanOrEqual(0)
  expect(box!.y).toBeGreaterThanOrEqual(0)
  expect(box!.x + box!.width).toBeLessThanOrEqual(viewport!.width + 1)
})

test('every route names itself with exactly one h1', async ({ page }) => {
  // `docs/47` §7 and `docs/49` §6: one `<h1>` per route, and it names the page.
  // Settings had none of its own for months — the only heading was the word
  // "Settings" in the navigation — and no source-level rule can see that,
  // because the element existed; it was in the wrong place.
  // The routes this fixture renders. `/settings/roles` and the task page need
  // a much fuller fixture than a *layout* suite should carry, and a stub grown
  // to satisfy them is a stub nobody trusts. They are covered by the jsdom
  // suite for behaviour; their geometry is not covered, and saying so is better
  // than a passing test that only proves the fixture answered.
  for (const path of ['/', '/settings/profile', '/reports']) {
    await page.goto(path)
    await page.waitForLoadState('networkidle')
    const headings = await page.evaluate(() =>
      [...document.querySelectorAll('h1')].map((h) => h.textContent?.trim() ?? ''),
    )
    expect(headings.length, `${path} has ${headings.length} h1s: ${headings.join(', ')}`).toBe(1)
    expect(headings[0]?.length ?? 0).toBeGreaterThan(0)
  }
})

test('one scrolling region per route', async ({ page }) => {
  // `docs/47` §3. A section cut off mid-control by a container it does not know
  // about is the failure this prevents, and it is invisible until you meet it.
  await page.goto('/settings/profile')
  await page.waitForLoadState('networkidle')
  const scrollers = await page.evaluate(() => {
    const out: string[] = []
    for (const el of document.querySelectorAll('body *')) {
      const style = getComputedStyle(el)
      const scrollsY = ['auto', 'scroll'].includes(style.overflowY)
      if (scrollsY && el.scrollHeight > el.clientHeight + 1) {
        out.push(el.className.toString().slice(0, 40) || el.tagName)
      }
    }
    return out
  })
  expect(scrollers.length, `more than one scroller: ${scrollers.join(', ')}`).toBeLessThanOrEqual(1)
})
