/**
 * Geometry, in a real browser (`docs/47` §9).
 *
 * Every assertion here is about something jsdom cannot see: whether an element
 * is on the screen, how wide it is, what order it is in, and how big it is under
 * a finger. Each one corresponds to a defect that shipped — all of them passed
 * `tsc`, `eslint` and the jsdom suite on the way out.
 */
import { expect, test } from '@playwright/test'

import { stubApi, stubApiWithoutWorkspace } from './stub'

test.beforeEach(async ({ page }) => {
  await stubApi(page)
})

/** Nothing may push the page sideways. `docs/47` §5: no horizontal scrolling. */
for (const path of ['/', '/board', '/settings/profile', '/settings/roles', '/reports']) {
  test(`${path} never scrolls sideways`, async ({ page }) => {
    // Audit item 2, closed. This carried `test.fail(isMobile, …)` until the
    // header stopped insisting on 260 px of search and 113 px of theme label,
    // and the bottom bar stopped sizing eight destinations by their words. The
    // marker is gone because it did its job: the fix made this report an
    // unexpected pass, which is the only signal that reliably gets a stale
    // expectation deleted.
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
  // Audit item 3, closed. The stacked row landed in #79 and then broke
  // silently: the mobile layout placed cells by `nth-child`, and adding Status
  // and Assignee columns shifted every one of those selectors two places, so
  // Due and Updated were auto-placed on top of the title. The row rendered as
  // overlapping text at 390 px and nothing in the source looked wrong.
  //
  // The two compositions get different assertions because they are different
  // layouts, and collapsing them into one weaker rule would have meant losing
  // the desktop guarantee to make the phone pass. On a desk the row is a table
  // and the title must be the widest *column*; on a phone it is a stacked
  // summary where several cells legitimately span the whole row, so what
  // matters is that the title spans it too rather than being squeezed by the
  // fixed columns beside it.
  await page.goto('/')
  // Scoped to a row. The column *heading* carries the same class and is
  // `display: none` on a phone, so an unscoped locator finds the hidden `<th>`
  // and reports the row as invisible — a failure that says nothing about the
  // row it is meant to be measuring.
  const title = page.locator('.list__row .list__cell--title').first()
  await expect(title).toBeVisible()

  const measured = await page.evaluate(() => {
    const row = document.querySelector('.list__row')
    if (row === null) return null
    return {
      rowWidth: Math.round(row.getBoundingClientRect().width),
      cells: [...row.querySelectorAll('.list__cell')].map((cell) => ({
        title: cell.classList.contains('list__cell--title'),
        width: Math.round(cell.getBoundingClientRect().width),
      })),
    }
  })
  expect(measured).not.toBeNull()
  const titleWidth = measured!.cells.find((cell) => cell.title)?.width ?? 0

  if (isMobile) {
    // The title owns its own line. The failure this catches is the one that
    // shipped: fixed columns eating the row until the title is a sliver, or
    // cells landing on top of each other.
    expect(
      titleWidth,
      `title ${titleWidth}px of a ${measured!.rowWidth}px row`,
    ).toBeGreaterThanOrEqual(measured!.rowWidth * 0.9)
  } else {
    // Every other column is a detail; the title is the row, and it is the one
    // that must never be what shrinks.
    const widest = Math.max(...measured!.cells.filter((c) => !c.title).map((c) => c.width))
    expect(titleWidth, `title ${titleWidth}px vs widest other column ${widest}px`).toBeGreaterThan(
      widest,
    )
  }
  // And wide enough to read a title in, not merely wider than a date.
  expect(titleWidth).toBeGreaterThan(200)
})

test('a list row contains its own cells', async ({ page }) => {
  // The assertion that was missing, and the reason audit item 3 stayed broken
  // through a passing suite: the width checks above were all satisfied while
  // the row rendered as four lines of text on top of each other.
  //
  // The cause was a fixed `height` on every virtualized row, taken from one
  // `ROW_HEIGHT` constant that is true of the desktop table and false of the
  // narrow stacked summary. Cells spilled out of a 40 px box — measurably, at
  // y = -2 to y = 60 — and nothing that measured *widths* could see it.
  await page.goto('/')
  await page.waitForLoadState('networkidle')

  const spills = await page.evaluate(() => {
    const out: string[] = []
    for (const row of document.querySelectorAll('.list__row')) {
      const box = row.getBoundingClientRect()
      for (const cell of row.querySelectorAll('.list__cell')) {
        const inner = cell.getBoundingClientRect()
        if (inner.height === 0) continue
        if (inner.top < box.top - 1 || inner.bottom > box.bottom + 1) {
          out.push(
            `${String(cell.className).slice(0, 30)} spans ${Math.round(inner.top - box.top)}..${Math.round(inner.bottom - box.top)} of a ${Math.round(box.height)}px row`,
          )
        }
      }
    }
    return out
  })
  expect(spills, spills.join('; ')).toEqual([])
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

test('the list shows status and assignee, and filters on the column', async ({
  page,
  isMobile,
}) => {
  // Desktop only: the narrow composition is a stacked summary with no column
  // headings at all, so there is nothing here to hang a column filter on.
  test.skip(isMobile, 'the narrow row is a stacked summary, not a table')
  // Both columns were absent, and they are the two fields people scan a list
  // for: what state is this in, and whose is it. A list that cannot say either
  // has to be opened row by row to answer them.
  await page.goto('/')
  await page.waitForLoadState('networkidle')

  const headings = await page.evaluate(() =>
    [...document.querySelectorAll('.list__head th')].map((th) => th.textContent?.trim() ?? ''),
  )
  expect(headings.join(' ')).toContain('Status')
  expect(headings.join(' ')).toContain('Assignee')

  // And the filter is on the column it narrows, not in a toolbar somewhere
  // else. Hidden until the heading is hovered, so it is found by hovering.
  // The Type column, because its options are a closed set that needs no
  // project scope — a status list belongs to one project's workflow, and at
  // workspace scope there is nothing to offer.
  const type = page.locator('.colhead', { hasText: 'Type' }).first()
  await type.hover()
  const funnel = type.locator('.colhead__filter')
  await expect(funnel).toBeVisible()

  // And the menu it opens is actually *on top*. `.list` sets `overflow:
  // hidden`, so a menu positioned inside the table was clipped by it and
  // appeared behind the rows with nothing visible — which no z-index fixes,
  // because a clipped box is not a stacking problem. `elementFromPoint` is the
  // only honest way to ask "is this painted here": a visible box behind an
  // opaque one is still `toBeVisible`.
  await funnel.click()
  const menu = await page.evaluate(() => {
    const el = document.querySelector('.colhead__menu')
    if (el === null) return null
    const box = el.getBoundingClientRect()
    const hit = document.elementFromPoint(box.left + box.width / 2, box.top + 20)
    const list = document.querySelector('.list')
    return {
      // The mechanism: `.list` sets `overflow: hidden`, so a menu positioned
      // *inside* the table is clipped by it — which is not a stacking problem
      // and no z-index fixes it. `fixed` is what takes it out of that
      // containing block, so the fix is what is asserted, not a symptom that
      // only appears once the list is long enough to clip.
      position: getComputedStyle(el).position,
      paintedOnTop: hit !== null && el.contains(hit),
      // And it is allowed to extend past the table, which is the thing the
      // clipping prevented.
      escapesTheTable: list !== null && box.bottom > list.getBoundingClientRect().bottom,
    }
  })
  expect(menu?.position).toBe('fixed')
  expect(menu?.paintedOnTop).toBe(true)
  expect(menu?.escapesTheTable).toBe(true)
})

/**
 * The rail and the header do not move.
 *
 * The defect this holds shut: `.shell__main` had no base rule at all, so it was
 * `overflow: visible` and `display: block`. Its content grew the grid row, grew
 * the document, and the *whole application* scrolled — the sidebar slid up out
 * of view and the header slid away with it, on every route long enough to need
 * scrolling. Two other rules in the same stylesheet already described
 * `.shell__main` as `hidden`, and the narrow layout overrode it, so the one
 * composition nobody had a rule for was the one everybody uses.
 *
 * Asserted on the document rather than on the shell: "does the page scroll" is
 * the user-visible fact, and it stays true however the panes are rearranged.
 */
for (const path of ['/', '/board', '/reports', '/dashboards/project-health', '/settings/roles']) {
  test(`${path} scrolls its content, not the whole application`, async ({ page, isMobile }) => {
    // On a phone the page *is* the scrolling region by design (`docs/47` §3),
    // so this invariant is a desktop one.
    test.skip(isMobile, 'the narrow composition scrolls the page on purpose')
    await page.goto(path)
    await page.waitForLoadState('networkidle')

    const result = await page.evaluate(() => {
      // Tall content, injected. Waiting for a route to be long enough on its
      // own makes the test a hostage to the fixtures: with the shell rule
      // deleted only one of these five routes had enough stub rows to overflow,
      // so four of them would have gone on passing through the regression. The
      // invariant is "however tall the content, the application does not
      // scroll", so the test supplies the height.
      const host = document.querySelector('.view__body') ?? document.querySelector('.shell__main')
      const filler = document.createElement('div')
      filler.style.height = '3000px'
      host?.appendChild(filler)

      const doc = document.documentElement
      return {
        documentScrolls: doc.scrollHeight > doc.clientHeight + 1,
        // And the content is reachable rather than merely clipped: something
        // inside the shell has to have become the scroller.
        innerScroller:
          [...document.querySelectorAll('.shell__main, .shell__main *')].some(
            (el) => el.scrollHeight > el.clientHeight + 1,
          ),
      }
    })

    expect(result.documentScrolls).toBe(false)
    expect(result.innerScroller).toBe(true)
  })
}

test('a person who belongs to nothing can start a workspace', async ({ page }) => {
  // Audit item 10. The first-run screen was a sentence with no control on it —
  // "ask an owner for an invitation" — while `POST /api/v1/workspaces` existed
  // the whole time and the client never called it. Someone signing up as the
  // first person in their organisation had no way forward at all.
  await stubApiWithoutWorkspace(page)
  await page.goto('/')
  await page.waitForLoadState('networkidle')

  await expect(page.getByRole('heading', { name: 'Start a workspace' })).toBeVisible()

  const name = page.getByLabel('Workspace name')
  await name.fill('Acme, Inc.')

  // The address is offered, not demanded, and it is offered as something the
  // server will accept — a suggestion that fails validation is a rejection the
  // person did not earn.
  await expect(page.getByLabel('Address')).toHaveValue('acme-inc')
  await expect(page.getByRole('button', { name: 'Create workspace' })).toBeEnabled()
})
