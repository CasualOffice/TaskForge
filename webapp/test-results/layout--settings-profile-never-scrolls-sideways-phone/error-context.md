# Instructions

- Following Playwright test failed.
- Explain why, be concise, respect Playwright best practices.
- Provide a snippet of code with the fix, if possible.

# Test info

- Name: layout.spec.ts >> /settings/profile never scrolls sideways
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
      - generic [ref=e57]:
        - navigation "Settings" [ref=e58]:
          - generic [ref=e59]:
            - heading "Account" [level=2] [ref=e60]
            - list "Account" [ref=e61]:
              - listitem [ref=e62]:
                - link "Your profile" [ref=e63] [cursor=pointer]:
                  - /url: /settings/profile
          - generic [ref=e64]:
            - heading "Workspace" [level=2] [ref=e65]
            - list "Workspace" [ref=e66]:
              - listitem [ref=e67]:
                - link "General" [ref=e68] [cursor=pointer]:
                  - /url: /settings/workspace
              - listitem [ref=e69]:
                - link "Members" [ref=e70] [cursor=pointer]:
                  - /url: /settings/members
              - listitem [ref=e71]:
                - link "Teams" [ref=e72] [cursor=pointer]:
                  - /url: /settings/teams
              - listitem [ref=e73]:
                - link "Roles" [ref=e74] [cursor=pointer]:
                  - /url: /settings/roles
              - listitem [ref=e75]:
                - link "Workflow" [ref=e76] [cursor=pointer]:
                  - /url: /settings/workflow
              - listitem [ref=e77]:
                - link "Environments" [ref=e78] [cursor=pointer]:
                  - /url: /settings/environments
              - listitem [ref=e79]:
                - link "Tags" [ref=e80] [cursor=pointer]:
                  - /url: /settings/tags
        - generic [ref=e81]:
          - generic [ref=e82]:
            - generic [ref=e84]:
              - heading "Your profile" [level=1] [ref=e85]
              - paragraph [ref=e86]: Signed in as test@example.test. The address cannot be changed here.
            - generic [ref=e87]:
              - paragraph [ref=e88]:
                - generic [ref=e89]: Display name
                - textbox "Display name" [ref=e92]: Test Person
              - paragraph [ref=e93]:
                - generic [ref=e94]: Time zone
                - textbox "Time zone" [ref=e97]:
                  - /placeholder: Asia/Calcutta
                  - text: Europe/London
                - generic [ref=e98]: Relative dates — today, overdue, next week — are resolved in this zone. Empty means UTC.
              - button "Save profile" [disabled] [ref=e99]
          - generic [ref=e101]:
            - generic [ref=e103]:
              - heading "Password" [level=2] [ref=e104]
              - paragraph [ref=e105]: Changing it signs out every other session, including your other devices. This one stays.
            - generic [ref=e106]:
              - paragraph [ref=e107]:
                - generic [ref=e108]: Current password
                - textbox "Current password" [ref=e111]
              - paragraph [ref=e112]:
                - generic [ref=e113]: New password
                - textbox "New password" [ref=e116]
                - generic [ref=e117]: At least 12 characters.
              - paragraph [ref=e118]:
                - generic [ref=e119]: New password again
                - textbox "New password again" [ref=e122]
              - button "Change password" [disabled] [ref=e123]
          - generic [ref=e125]:
            - generic [ref=e127]:
              - heading "Where you are signed in" [level=2] [ref=e128]
              - paragraph [ref=e129]: Every live session on this account. Signing one out takes effect on its next request.
            - table [ref=e131]:
              - rowgroup [ref=e132]:
                - row [ref=e133]:
                  - columnheader "Client" [ref=e134]
                  - columnheader "Last seen" [ref=e135]
                  - columnheader "Signed in with" [ref=e136]
                  - columnheader "Actions" [ref=e137]
              - rowgroup [ref=e139]:
                - row [ref=e140]:
                  - cell "Firefox on a laptop this session no address recorded" [ref=e141]:
                    - generic [ref=e142]:
                      - text: Firefox on a laptop
                      - generic [ref=e143]: this session
                    - generic [ref=e144]: no address recorded
                  - cell [ref=e145]:
                    - time [ref=e146]: 1/2/2026, 5:30:00 AM
                  - cell "password" [ref=e147]
                  - cell [ref=e148]
  - status [ref=e149]
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