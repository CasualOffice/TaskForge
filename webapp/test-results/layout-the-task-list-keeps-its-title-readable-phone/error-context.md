# Instructions

- Following Playwright test failed.
- Explain why, be concise, respect Playwright best practices.
- Provide a snippet of code with the fix, if possible.

# Test info

- Name: layout.spec.ts >> the task list keeps its title readable
- Location: e2e/layout.spec.ts:37:1

# Error details

```
Error: expect(locator).toBeVisible() failed

Locator:  locator('.list__cell--title').first()
Expected: visible
Received: hidden
Timeout:  5000ms

Call log:
  - Expect "toBeVisible" with timeout 5000ms
  - waiting for locator('.list__cell--title').first()
    14 × locator resolved to <th scope="col" class="list__cell list__cell--title">Title</th>
       - unexpected value "hidden"

```

```yaml
- link "Skip to content":
  - /url: "#main"
- banner:
  - link "TaskForge — Home":
    - /url: /home
    - text: TaskForge
  - button "⌘K"
  - button "Light theme. Activate to change.": Light theme
  - 'button "Your account: Test Person"': TP
- navigation "Primary":
  - list:
    - listitem:
      - link "Home":
        - /url: /home
    - listitem:
      - link "My work":
        - /url: /my-work
    - listitem:
      - link "All tasks":
        - /url: /
    - listitem:
      - link "Board":
        - /url: /board
    - listitem:
      - link "Environments":
        - /url: /environments
    - listitem:
      - link "Reports":
        - /url: /reports
  - list:
    - listitem:
      - link "Settings":
        - /url: /settings
- main:
  - region "List":
    - navigation "Breadcrumb":
      - list:
        - listitem: Acme
        - listitem: /List
    - heading "List" [level=1]
    - text: 1 shown
    - button "New task"
    - button "Views"
    - text: Project
    - combobox "Project":
      - option "All projects" [selected]
      - option "ONB — Onboarding"
    - text: Search tasks
    - searchbox "Search tasks"
    - button "Priority"
    - button "Type"
    - text: Assignee
    - combobox "Assignee":
      - option "Anyone" [selected]
      - option "Me"
      - option "Unassigned"
      - option
    - text: Due
    - combobox "Due":
      - option "Any due date" [selected]
      - option "Overdue"
      - option "Due today or earlier"
      - option "Due within a week"
      - option "Due within a month"
    - button "More"
    - text: Group by
    - combobox "Group by":
      - option "No grouping" [selected]
      - option "Group by status — needs a project" [disabled]
      - option "Group by state"
      - option "Group by type"
      - option "Group by priority"
    - text: Sort by
    - combobox "Sort by":
      - option "Last updated ↓" [selected]
      - option "Last updated ↑"
      - option "Created ↓"
      - option "Created ↑"
      - option "Due date ↓"
      - option "Due date ↑"
      - option "Priority ↓"
      - option "Priority ↑"
      - option "Board order ↓"
      - option "Board order ↑"
      - option "Identifier ↓"
      - option "Identifier ↑"
    - table:
      - rowgroup:
        - row "Bug ONB-12 Fix the mobile task layout so the title survives a narrow screen instead of collapsing High — Jan 2, 2026":
          - cell "Bug"
          - cell "ONB-12":
            - link "ONB-12":
              - /url: /tasks/019fe000-0000-7000-8000-000000000004
          - cell "Fix the mobile task layout so the title survives a narrow screen instead of collapsing":
            - link "Fix the mobile task layout so the title survives a narrow screen instead of collapsing":
              - /url: /tasks/019fe000-0000-7000-8000-000000000004
          - cell "High"
          - cell "—"
          - cell "Jan 2, 2026":
            - time: Jan 2, 2026
- status
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
  33  |     expect(overflow.scrollWidth).toBeLessThanOrEqual(overflow.clientWidth + 1)
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
> 46  |   await expect(title).toBeVisible()
      |                       ^ Error: expect(locator).toBeVisible() failed
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