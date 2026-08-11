/**
 * The dashboard's geometry (`docs/38`).
 *
 * A grid of charts is the surface jsdom is least able to check: it has no
 * layout, so a four-column grid on a 390 px phone, a bar wider than the tile it
 * sits in, and an SVG that collapsed to zero height all look identical to a
 * passing unit test. Everything asserted here is a measurement.
 */
import { expect, test } from '@playwright/test'

import { stubApi } from './stub'

test.beforeEach(async ({ page }) => {
  await stubApi(page)
})

const DASHBOARDS = ['my-work', 'project-health', 'team-workload', 'quality']

test('every dashboard renders its tiles', async ({ page }) => {
  for (const id of DASHBOARDS) {
    await page.goto(`/dashboards/${id}`)
    await page.waitForLoadState('networkidle')
    // Not "some tiles": a dashboard that silently drops half its tiles because
    // one measure failed to parse still looks fine in a screenshot.
    await expect(page.locator('.tile')).not.toHaveCount(0)
    await expect(page.locator('.tile__pending')).toHaveCount(0)
  }
})

test('/dashboards lands on a dashboard rather than a chooser', async ({ page }) => {
  await page.goto('/dashboards')
  await page.waitForLoadState('networkidle')
  await expect(page).toHaveURL(/\/dashboards\/my-work/)
  await expect(page.locator('.tile').first()).toBeVisible()
})

for (const id of DASHBOARDS) {
  test(`${id} never scrolls sideways`, async ({ page }) => {
    // This inherited audit item 2 — at 390 px every overflowing element was
    // shell chrome, not a `dash__`, `tile` or `chart__` class — and it is now
    // fixed at the shell, so the marker is gone and this asserts for real on
    // both compositions. The desktop half caught a defect of its own on its
    // first run: the hidden data table each chart carries was pushing the
    // document 62 px wider than the viewport.
    await page.goto(`/dashboards/${id}`)
    await page.waitForLoadState('networkidle')
    const overflow = await page.evaluate(() => ({
      scrollWidth: document.documentElement.scrollWidth,
      clientWidth: document.documentElement.clientWidth,
    }))
    expect(overflow.scrollWidth).toBeLessThanOrEqual(overflow.clientWidth + 1)
  })
}

test('no chart draws outside the tile that owns it', async ({ page }) => {
  // The failure this catches: a bar at 100% of a track that is itself wider
  // than its column, which reads as a chart bleeding into its neighbour.
  await page.goto('/dashboards/team-workload')
  await page.waitForLoadState('networkidle')

  const escapes = await page.evaluate(() => {
    const out: string[] = []
    for (const tile of document.querySelectorAll('.tile')) {
      const box = tile.getBoundingClientRect()
      for (const child of tile.querySelectorAll('.chart__fill, .chart__svg, .chart__stack, .chart__ring')) {
        const inner = child.getBoundingClientRect()
        if (inner.right > box.right + 1 || inner.left < box.left - 1) {
          out.push(`${child.className} in ${tile.className}`)
        }
      }
    }
    return out
  })
  expect(escapes).toEqual([])
})

test('the grid collapses to one column on a phone', async ({ page, isMobile }) => {
  test.skip(!isMobile, 'the collapse is what is being measured')
  await page.goto('/dashboards/project-health')
  await page.waitForLoadState('networkidle')

  // Measured rather than read off the CSS: `grid-column: span 2` inside a
  // one-column grid is the exact mistake that produces a page wider than the
  // phone, and the computed style would still say "1fr".
  //
  // The breakdowns only. Signals deliberately stay two-up on a phone: they are
  // why the page exists, and four single-column cards would push every chart
  // off the first screen — a summary you have to scroll to reach is not one.
  const lefts = await page.evaluate(() =>
    [...document.querySelectorAll('.dash__grid .tile')].map((tile) =>
      Math.round(tile.getBoundingClientRect().left),
    ),
  )
  expect(new Set(lefts).size).toBe(1)
})

test('the signals stay two-up on a phone', async ({ page, isMobile }) => {
  test.skip(!isMobile, 'the two-up rule is a phone rule')
  await page.goto('/dashboards/project-health')
  await page.waitForLoadState('networkidle')

  const lefts = await page.evaluate(() =>
    [...document.querySelectorAll('.dash__signals .tile')].map((tile) =>
      Math.round(tile.getBoundingClientRect().left),
    ),
  )
  expect(new Set(lefts).size).toBe(2)
})

test('a line chart has real height', async ({ page }) => {
  // An SVG with `height: auto` and `preserveAspectRatio: none` collapses to
  // nothing and takes the trend with it, silently.
  await page.goto('/dashboards/project-health')
  await page.waitForLoadState('networkidle')
  const svg = page.locator('.chart__svg').first()
  await expect(svg).toBeVisible()
  const box = await svg.boundingBox()
  expect(box?.height ?? 0).toBeGreaterThan(60)
  expect(box?.width ?? 0).toBeGreaterThan(120)
})

test('every chart carries its numbers as a table', async ({ page }) => {
  // `docs/47`: the drawing is decoration, the table is the content. A chart
  // that lost its table is a chart a screen reader cannot read at all — and
  // nothing on screen would look wrong.
  await page.goto('/dashboards/team-workload')
  await page.waitForLoadState('networkidle')

  const missing = await page.evaluate(() =>
    [...document.querySelectorAll('.chart')]
      .filter((chart) => chart.querySelector('table') === null)
      .map((chart) => chart.parentElement?.parentElement?.className ?? '?'),
  )
  expect(missing).toEqual([])
})

test('a signal opens the tasks it counted', async ({ page }) => {
  // The whole point of the surface, and what the first version of it lacked:
  // reading "Overdue 3" and then having to rebuild that filter by hand in the
  // list is where a dashboard stops being useful. Asserted against the address,
  // not the rows, because the rows come from the stub and asserting a stub
  // asserts nothing — the address is what the client actually decided.
  await page.goto('/dashboards/team-workload')
  await page.waitForLoadState('networkidle')

  const overdue = page.locator('.tile--open', { hasText: 'Overdue' }).first()
  await expect(overdue).toBeVisible()
  await overdue.click()

  await expect(page).toHaveURL(/state=BACKLOG%2CPLANNED%2CACTIVE/)
  await expect(page).toHaveURL(/due=%3C%40today/)
})

test('the signals band is the first thing on the page', async ({ page }) => {
  // Hierarchy, measured: if a breakdown ever renders above the signals, the
  // page has stopped answering "is anything wrong" first and gone back to being
  // a wall of equal cards.
  await page.goto('/dashboards/project-health')
  await page.waitForLoadState('networkidle')

  const signalTop = await page.locator('.dash__signals .tile').first().boundingBox()
  const chartTop = await page.locator('.dash__grid .tile').first().boundingBox()
  expect(signalTop?.y ?? 0).toBeLessThan(chartTop?.y ?? 0)
})

test('a duration tile is not a link', async ({ page }) => {
  // "Cycle time by project" measures completed work; a list behind it would
  // have a row count with no relationship to the number above it.
  await page.goto('/dashboards/project-health')
  await page.waitForLoadState('networkidle')

  const cycle = page.locator('.tile', { hasText: 'Cycle time by project' }).first()
  await expect(cycle).toBeVisible()
  expect(await cycle.evaluate((el) => el.tagName)).toBe('SECTION')
})

test('the dashboard switcher is reachable under a finger', async ({ page, isMobile }) => {
  test.skip(!isMobile, 'the 44 px tier is a touch target rule')
  await page.goto('/dashboards/my-work')
  await page.waitForLoadState('networkidle')

  const tabs = page.locator('.dash__tab')
  const count = await tabs.count()
  expect(count).toBeGreaterThan(0)
  for (let i = 0; i < count; i += 1) {
    const box = await tabs.nth(i).boundingBox()
    expect(box?.height ?? 0).toBeGreaterThanOrEqual(44)
  }
})
