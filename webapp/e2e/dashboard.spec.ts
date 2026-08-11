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
  test(`${id} never scrolls sideways`, async ({ page, isMobile }) => {
    // Audit item 2 again, and it is the *shell* rather than this surface: at
    // 390 px every overflowing element is chrome — `shell__search`,
    // `shell__account`, `side__link`, the account popover — and not one carries
    // a `dash__`, `tile` or `chart__` class. Measured, not assumed, because the
    // honest thing to do with an inherited failure is to prove it is inherited.
    // The desktop half of this test is what guards the grid itself, and it
    // caught a real defect on its first run: the hidden data table each chart
    // carries was pushing the document 62 px wider than the viewport.
    test.fail(isMobile, 'audit item 2 — the narrow shell still overflows')
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
      for (const child of tile.querySelectorAll('.chart__fill, .chart__svg, .chart__stack')) {
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
  const lefts = await page.evaluate(() =>
    [...document.querySelectorAll('.tile')].map((tile) =>
      Math.round(tile.getBoundingClientRect().left),
    ),
  )
  expect(new Set(lefts).size).toBe(1)
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
