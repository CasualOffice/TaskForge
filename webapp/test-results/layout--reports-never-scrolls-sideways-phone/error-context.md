# Instructions

- Following Playwright test failed.
- Explain why, be concise, respect Playwright best practices.
- Provide a snippet of code with the fix, if possible.

# Test info

- Name: layout.spec.ts >> /reports never scrolls sideways
- Location: e2e/layout.spec.ts:19:3

# Error details

```
Error: expect(received).toBeLessThanOrEqual(expected)

Expected: <= 391
Received:    583
```

# Page snapshot

```yaml
- generic [ref=e2]:
  - generic [ref=e3]:
    - link "Skip to content" [ref=e4] [cursor=pointer]:
      - /url: "#main"
    - banner [ref=e5]:
      - link "TaskForge — Home" [ref=e6] [cursor=pointer]:
        - /url: /home
        - generic [ref=e7]: TaskForge
      - button "⌘K" [ref=e8] [cursor=pointer]:
        - generic [ref=e9]: search
      - button "Light theme. Activate to change." [ref=e12] [cursor=pointer]:
        - generic [ref=e13]: light_mode
        - generic [ref=e14]: Light theme
      - 'button "Your account: Test Person" [ref=e16] [cursor=pointer]':
        - generic "Test Person" [ref=e17]: TP
    - navigation "Primary" [ref=e18]:
      - list [ref=e19]:
        - listitem [ref=e20]:
          - link "Home" [ref=e21] [cursor=pointer]:
            - /url: /home
            - generic [ref=e22]: home
        - listitem [ref=e25]:
          - link "My work" [ref=e26] [cursor=pointer]:
            - /url: /my-work
            - generic [ref=e27]: person
        - listitem [ref=e30]:
          - link "All tasks" [ref=e31] [cursor=pointer]:
            - /url: /
            - generic [ref=e32]: checklist
        - listitem [ref=e35]:
          - link "Board" [ref=e36] [cursor=pointer]:
            - /url: /board
            - generic [ref=e37]: view_kanban
        - listitem [ref=e40]:
          - link "Environments" [ref=e41] [cursor=pointer]:
            - /url: /environments
            - generic [ref=e42]: lan
        - listitem [ref=e45]:
          - link "Reports" [ref=e46] [cursor=pointer]:
            - /url: /reports
            - generic [ref=e47]: monitoring
      - list [ref=e50]:
        - listitem [ref=e51]:
          - link "Settings" [ref=e52] [cursor=pointer]:
            - /url: /settings
            - generic [ref=e53]: settings
    - main [ref=e56]:
      - region [ref=e57]:
        - generic [ref=e58]:
          - navigation "Breadcrumb" [ref=e59]:
            - list [ref=e60]:
              - listitem [ref=e61]: Acme
              - listitem [ref=e62]: /Reports
          - generic [ref=e63]:
            - heading "Reports" [level=1] [ref=e64]
            - generic [ref=e65]: 0 tasks
        - generic [ref=e66]:
          - button "Views" [ref=e68] [cursor=pointer]:
            - generic [ref=e70]: expand_more
          - generic [ref=e71]: Project
          - generic [ref=e73]:
            - combobox "Project" [ref=e74] [cursor=pointer]:
              - option "All projects" [selected]
              - option "ONB — Onboarding"
            - generic: arrow_drop_down
          - generic [ref=e75]:
            - generic [ref=e76]: Search tasks
            - searchbox "Search tasks" [ref=e79]
            - button "Priority" [ref=e81] [cursor=pointer]:
              - generic [ref=e83]: expand_more
            - button "Type" [ref=e85] [cursor=pointer]:
              - generic [ref=e87]: expand_more
            - generic [ref=e88]: Assignee
            - generic [ref=e90]:
              - combobox "Assignee" [ref=e91] [cursor=pointer]:
                - option "Anyone" [selected]
                - option "Me"
                - option "Unassigned"
                - option
              - generic: arrow_drop_down
            - generic [ref=e92]: Due
            - generic [ref=e94]:
              - combobox "Due" [ref=e95] [cursor=pointer]:
                - option "Any due date" [selected]
                - option "Overdue"
                - option "Due today or earlier"
                - option "Due within a week"
                - option "Due within a month"
              - generic: arrow_drop_down
            - button "More" [ref=e97] [cursor=pointer]:
              - generic [ref=e99]: expand_more
          - generic [ref=e100]: What to measure
          - generic [ref=e102]:
            - combobox "What to measure" [ref=e103] [cursor=pointer]:
              - option "How many" [selected]
              - option "Cycle time (median)"
              - option "Cycle time (90th percentile)"
              - option "Lead time (median)"
              - option "Throughput"
            - generic: arrow_drop_down
          - generic [ref=e104]: Group the count by
          - generic [ref=e106]:
            - combobox "Group the count by" [ref=e107] [cursor=pointer]:
              - option "by type" [selected]
              - option "by priority"
              - option "by state"
              - option "by project"
              - option "by team"
              - option "by assignee"
              - option "by reporter"
            - generic: arrow_drop_down
        - generic [ref=e109]:
          - paragraph [ref=e110]: Nothing matches
          - paragraph [ref=e111]: No task in the projects you can see matches these filters. Widen them in the bar above.
  - status [ref=e112]
```

# Test source

```ts
  1   | /**
  2   |  * Geometry, in a real browser (`docs/47` §9).
  3   |  *
  4   |  * Every assertion here is about something jsdom cannot see: whether an element
  5   |  * is on the screen, how wide it is, what order it is in, and how big it is under
  6   |  * a finger. Each one corresponds to a defect that shipped — all of them passed
  7   |  * `tsc`, `eslint` and the jsdom suite on the way out.
  8   |  */
  9   | import { expect, test } from '@playwright/test'
  10  | 
  11  | import { stubApi } from './stub'
  12  | 
  13  | test.beforeEach(async ({ page }) => {
  14  |   await stubApi(page)
  15  | })
  16  | 
  17  | /** Nothing may push the page sideways. `docs/47` §5: no horizontal scrolling. */
  18  | for (const path of ['/', '/board', '/settings/profile', '/settings/roles', '/reports']) {
  19  |   test(`${path} never scrolls sideways`, async ({ page, isMobile }) => {
  20  |     // Audit item 2, still open: the shell is wider than a 390 px viewport on
  21  |     // several routes. `test.fail` rather than `skip` — the assertion runs, and
  22  |     // the day someone fixes the layout this reports an *unexpected pass* and
  23  |     // makes them delete this line. A skipped test is a test nobody removes.
  24  |     test.fail(isMobile, 'audit item 2 — the narrow shell still overflows')
  25  |     await page.goto(path)
  26  |     await page.waitForLoadState('networkidle')
  27  |     const overflow = await page.evaluate(() => ({
  28  |       scrollWidth: document.documentElement.scrollWidth,
  29  |       clientWidth: document.documentElement.clientWidth,
  30  |     }))
  31  |     // One pixel of tolerance for sub-pixel rounding, and no more: two is a
  32  |     // layout that is actually wider than the screen.
> 33  |     expect(overflow.scrollWidth).toBeLessThanOrEqual(overflow.clientWidth + 1)
      |                                  ^ Error: expect(received).toBeLessThanOrEqual(expected)
  34  |   })
  35  | }
  36  | 
  37  | test('the task list keeps its title readable', async ({ page, isMobile }) => {
  38  |   // Audit item 3 on a phone. The stacked row landed in #79 but is unverified
  39  |   // there — this is what verifies it, and it does not pass yet.
  40  |   test.fail(isMobile, 'audit item 3 — the stacked row does not hold at 390 px')
  41  |   // The reported failure: 492 px of fixed columns before the title's `1fr`, so
  42  |   // on a phone the fixed columns ate the row and the title — the one thing
  43  |   // anyone scans for — was the column that collapsed.
  44  |   await page.goto('/')
  45  |   const title = page.locator('.list__cell--title').first()
  46  |   await expect(title).toBeVisible()
  47  | 
  48  |   // The invariant is not a fraction of the screen — a sidebar and five other
  49  |   // columns make that meaningless — it is that the title is the **widest**
  50  |   // thing in its row. Every other column is a detail; the title is the row, and
  51  |   // it is the one that must never be what shrinks.
  52  |   const widths = await page.evaluate(() => {
  53  |     const row = document.querySelector('.list__row')
  54  |     if (row === null) return null
  55  |     return [...row.querySelectorAll('.list__cell')].map((cell) => ({
  56  |       title: cell.classList.contains('list__cell--title'),
  57  |       width: Math.round(cell.getBoundingClientRect().width),
  58  |     }))
  59  |   })
  60  |   expect(widths).not.toBeNull()
  61  |   const titleWidth = widths!.find((cell) => cell.title)?.width ?? 0
  62  |   const widest = Math.max(...widths!.filter((cell) => !cell.title).map((cell) => cell.width))
  63  |   expect(titleWidth, `title ${titleWidth}px vs widest other column ${widest}px`).toBeGreaterThan(
  64  |     widest,
  65  |   )
  66  |   // And wide enough to read a title in, not merely wider than a date.
  67  |   expect(titleWidth).toBeGreaterThan(200)
  68  | })
  69  | 
  70  | test('the create form opens fully on screen', async ({ page }) => {
  71  |   // The reported failure: a 360 px panel right-aligned to a trigger near the
  72  |   // left edge extends off the screen, and an absolutely positioned surface is
  73  |   // clipped by any ancestor that hides overflow.
  74  |   await page.goto('/')
  75  |   await page.getByRole('button', { name: 'New task' }).click()
  76  |   const surface = page.locator('.pop__surface')
  77  |   await expect(surface).toBeVisible()
  78  |   const box = await surface.boundingBox()
  79  |   const viewport = page.viewportSize()
  80  |   expect(box).not.toBeNull()
  81  |   expect(box!.x).toBeGreaterThanOrEqual(0)
  82  |   expect(box!.y).toBeGreaterThanOrEqual(0)
  83  |   expect(box!.x + box!.width).toBeLessThanOrEqual(viewport!.width + 1)
  84  | })
  85  | 
  86  | test('every route names itself with exactly one h1', async ({ page }) => {
  87  |   // `docs/47` §7 and `docs/49` §6: one `<h1>` per route, and it names the page.
  88  |   // Settings had none of its own for months — the only heading was the word
  89  |   // "Settings" in the navigation — and no source-level rule can see that,
  90  |   // because the element existed; it was in the wrong place.
  91  |   // The routes this fixture renders. `/settings/roles` and the task page need
  92  |   // a much fuller fixture than a *layout* suite should carry, and a stub grown
  93  |   // to satisfy them is a stub nobody trusts. They are covered by the jsdom
  94  |   // suite for behaviour; their geometry is not covered, and saying so is better
  95  |   // than a passing test that only proves the fixture answered.
  96  |   for (const path of ['/', '/settings/profile', '/reports']) {
  97  |     await page.goto(path)
  98  |     await page.waitForLoadState('networkidle')
  99  |     const headings = await page.evaluate(() =>
  100 |       [...document.querySelectorAll('h1')].map((h) => h.textContent?.trim() ?? ''),
  101 |     )
  102 |     expect(headings.length, `${path} has ${headings.length} h1s: ${headings.join(', ')}`).toBe(1)
  103 |     expect(headings[0]?.length ?? 0).toBeGreaterThan(0)
  104 |   }
  105 | })
  106 | 
  107 | test('one scrolling region per route', async ({ page }) => {
  108 |   // `docs/47` §3. A section cut off mid-control by a container it does not know
  109 |   // about is the failure this prevents, and it is invisible until you meet it.
  110 |   await page.goto('/settings/profile')
  111 |   await page.waitForLoadState('networkidle')
  112 |   const scrollers = await page.evaluate(() => {
  113 |     const out: string[] = []
  114 |     for (const el of document.querySelectorAll('body *')) {
  115 |       const style = getComputedStyle(el)
  116 |       const scrollsY = ['auto', 'scroll'].includes(style.overflowY)
  117 |       if (scrollsY && el.scrollHeight > el.clientHeight + 1) {
  118 |         out.push(el.className.toString().slice(0, 40) || el.tagName)
  119 |       }
  120 |     }
  121 |     return out
  122 |   })
  123 |   expect(scrollers.length, `more than one scroller: ${scrollers.join(', ')}`).toBeLessThanOrEqual(1)
  124 | })
  125 | 
```