/**
 * Touch targets, measured (`docs/47` §2, WCAG 2.2 §2.5.8).
 *
 * The repository commits to a 44 px tier on coarse pointers. Nothing checked
 * it, and an audit found 13–25 controls per route below it — a number no
 * source-level linter can produce, because the size is a rendering fact.
 */
import { expect, test } from '@playwright/test'

import { stubApi } from './stub'

test.beforeEach(async ({ page }) => {
  await stubApi(page)
})

test.describe('on a coarse pointer', () => {
  test.skip(({ isMobile }) => !isMobile, 'the tier applies to touch, not to a mouse')

  for (const path of ['/', '/settings/profile']) {
    test(`${path} has no control under the 44 px tier`, async ({ page }) => {
      // Audit item 5, still open: 13–25 controls per route are under the tier.
      // Recorded as an expected failure so the number is visible in CI and the
      // fix reports an unexpected pass rather than silence.
      test.fail(true, 'audit item 5 — controls under the 44 px tier')
      await page.goto(path)
      await page.waitForLoadState('networkidle')

      const small = await page.evaluate(() => {
        const out: string[] = []
        const selector = 'a[href], button, select, input:not([type=hidden]), summary'
        for (const el of document.querySelectorAll(selector)) {
          const rect = el.getBoundingClientRect()
          // Only what is actually on the screen: a control inside a closed
          // disclosure has no size and is not a target yet.
          if (rect.width === 0 || rect.height === 0) continue
          if (rect.height < 44) {
            out.push(
              `${el.tagName}.${el.className.toString().slice(0, 24)} h=${Math.round(rect.height)}`,
            )
          }
        }
        return out
      })

      expect(small, `controls under 44 px: ${small.join(' | ')}`).toEqual([])
    })
  }
})
