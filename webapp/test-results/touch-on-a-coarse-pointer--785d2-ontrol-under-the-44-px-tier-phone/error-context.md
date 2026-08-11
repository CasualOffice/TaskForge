# Instructions

- Following Playwright test failed.
- Explain why, be concise, respect Playwright best practices.
- Provide a snippet of code with the fix, if possible.

# Test info

- Name: touch.spec.ts >> on a coarse pointer >> / has no control under the 44 px tier
- Location: e2e/touch.spec.ts:20:5

# Error details

```
Error: controls under 44 px: A.skip-link h=31 | A.shell__brand h=22 | BUTTON.shell__search h=32 | BUTTON. h=28 | BUTTON.shell__account h=38 | BUTTON. h=34 | BUTTON. h=34 | SELECT. h=34 | INPUT. h=23 | SELECT. h=34 | SELECT. h=34 | BUTTON. h=34 | SELECT. h=34 | SELECT. h=34 | A.list__open list__open--t h=38

expect(received).toEqual(expected) // deep equality

- Expected  -  1
+ Received  + 17

- Array []
+ Array [
+   "A.skip-link h=31",
+   "A.shell__brand h=22",
+   "BUTTON.shell__search h=32",
+   "BUTTON. h=28",
+   "BUTTON.shell__account h=38",
+   "BUTTON. h=34",
+   "BUTTON. h=34",
+   "SELECT. h=34",
+   "INPUT. h=23",
+   "SELECT. h=34",
+   "SELECT. h=34",
+   "BUTTON. h=34",
+   "SELECT. h=34",
+   "SELECT. h=34",
+   "A.list__open list__open--t h=38",
+ ]
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
              - listitem [ref=e62]: /List
          - generic [ref=e63]:
            - heading "List" [level=1] [ref=e64]
            - generic [ref=e65]: 1 shown
            - button "New task" [ref=e67] [cursor=pointer]
        - generic [ref=e69]:
          - button "Views" [ref=e71] [cursor=pointer]:
            - generic [ref=e73]: expand_more
          - generic [ref=e74]: Project
          - generic [ref=e76]:
            - combobox "Project" [ref=e77] [cursor=pointer]:
              - option "All projects" [selected]
              - option "ONB — Onboarding"
            - generic: arrow_drop_down
          - generic [ref=e78]:
            - generic [ref=e79]: Search tasks
            - searchbox "Search tasks" [ref=e82]
            - generic [ref=e83]: Assignee
            - generic [ref=e85]:
              - combobox "Assignee" [ref=e86] [cursor=pointer]:
                - option "Anyone" [selected]
                - option "Me"
                - option "Unassigned"
                - option
              - generic: arrow_drop_down
            - generic [ref=e87]: Due
            - generic [ref=e89]:
              - combobox "Due" [ref=e90] [cursor=pointer]:
                - option "Any due date" [selected]
                - option "Overdue"
                - option "Due today or earlier"
                - option "Due within a week"
                - option "Due within a month"
              - generic: arrow_drop_down
            - button "More" [ref=e92] [cursor=pointer]:
              - generic [ref=e94]: expand_more
          - generic [ref=e95]: Group by
          - generic [ref=e97]:
            - combobox "Group by" [ref=e98] [cursor=pointer]:
              - option "No grouping" [selected]
              - option "Group by status — needs a project" [disabled]
              - option "Group by state"
              - option "Group by type"
              - option "Group by priority"
            - generic: arrow_drop_down
          - generic [ref=e99]: Sort by
          - generic [ref=e101]:
            - combobox "Sort by" [ref=e102] [cursor=pointer]:
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
            - generic: arrow_drop_down
        - table [ref=e104]:
          - rowgroup [ref=e105]:
            - row [ref=e106]:
              - cell "Bug" [ref=e107]
              - cell [ref=e109]:
                - link "ONB-12" [ref=e110] [cursor=pointer]:
                  - /url: /tasks/019fe000-0000-7000-8000-000000000004
              - cell [ref=e111]:
                - link "Fix the mobile task layout so the title survives a narrow screen instead of collapsing" [ref=e112] [cursor=pointer]:
                  - /url: /tasks/019fe000-0000-7000-8000-000000000004
              - cell "Active" [ref=e113]
              - cell "—" [ref=e115]
              - cell "High" [ref=e116]
              - cell "—" [ref=e118]
              - cell [ref=e119]:
                - time [ref=e120]: Jan 2, 2026
  - status [ref=e121]
```

# Test source

```ts
  1  | /**
  2  |  * Touch targets, measured (`docs/47` §2, WCAG 2.2 §2.5.8).
  3  |  *
  4  |  * The repository commits to a 44 px tier on coarse pointers. Nothing checked
  5  |  * it, and an audit found 13–25 controls per route below it — a number no
  6  |  * source-level linter can produce, because the size is a rendering fact.
  7  |  */
  8  | import { expect, test } from '@playwright/test'
  9  | 
  10 | import { stubApi } from './stub'
  11 | 
  12 | test.beforeEach(async ({ page }) => {
  13 |   await stubApi(page)
  14 | })
  15 | 
  16 | test.describe('on a coarse pointer', () => {
  17 |   test.skip(({ isMobile }) => !isMobile, 'the tier applies to touch, not to a mouse')
  18 | 
  19 |   for (const path of ['/', '/settings/profile']) {
  20 |     test(`${path} has no control under the 44 px tier`, async ({ page }) => {
  21 |       // Audit item 5, still open: 13–25 controls per route are under the tier.
  22 |       // Recorded as an expected failure so the number is visible in CI and the
  23 |       // fix reports an unexpected pass rather than silence.
  24 |       test.fail(true, 'audit item 5 — controls under the 44 px tier')
  25 |       await page.goto(path)
  26 |       await page.waitForLoadState('networkidle')
  27 | 
  28 |       const small = await page.evaluate(() => {
  29 |         const out: string[] = []
  30 |         const selector = 'a[href], button, select, input:not([type=hidden]), summary'
  31 |         for (const el of document.querySelectorAll(selector)) {
  32 |           const rect = el.getBoundingClientRect()
  33 |           // Only what is actually on the screen: a control inside a closed
  34 |           // disclosure has no size and is not a target yet.
  35 |           if (rect.width === 0 || rect.height === 0) continue
  36 |           if (rect.height < 44) {
  37 |             out.push(
  38 |               `${el.tagName}.${el.className.toString().slice(0, 24)} h=${Math.round(rect.height)}`,
  39 |             )
  40 |           }
  41 |         }
  42 |         return out
  43 |       })
  44 | 
> 45 |       expect(small, `controls under 44 px: ${small.join(' | ')}`).toEqual([])
     |                                                                   ^ Error: controls under 44 px: A.skip-link h=31 | A.shell__brand h=22 | BUTTON.shell__search h=32 | BUTTON. h=28 | BUTTON.shell__account h=38 | BUTTON. h=34 | BUTTON. h=34 | SELECT. h=34 | INPUT. h=23 | SELECT. h=34 | SELECT. h=34 | BUTTON. h=34 | SELECT. h=34 | SELECT. h=34 | A.list__open list__open--t h=38
  46 |     })
  47 |   }
  48 | })
  49 | 
```