# Instructions

- Following Playwright test failed.
- Explain why, be concise, respect Playwright best practices.
- Provide a snippet of code with the fix, if possible.

# Test info

- Name: touch.spec.ts >> on a coarse pointer >> /settings/profile has no control under the 44 px tier
- Location: e2e/touch.spec.ts:20:5

# Error details

```
Error: controls under 44 px: A.skip-link h=31 | A.shell__brand h=22 | BUTTON.shell__search h=32 | BUTTON. h=28 | BUTTON.shell__account h=38 | INPUT. h=23 | INPUT. h=23 | BUTTON. h=34 | INPUT. h=23 | INPUT. h=23 | INPUT. h=23 | BUTTON. h=34

expect(received).toEqual(expected) // deep equality

- Expected  -  1
+ Received  + 14

- Array []
+ Array [
+   "A.skip-link h=31",
+   "A.shell__brand h=22",
+   "BUTTON.shell__search h=32",
+   "BUTTON. h=28",
+   "BUTTON.shell__account h=38",
+   "INPUT. h=23",
+   "INPUT. h=23",
+   "BUTTON. h=34",
+   "INPUT. h=23",
+   "INPUT. h=23",
+   "INPUT. h=23",
+   "BUTTON. h=34",
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
              - generic [ref=e88]:
                - generic [ref=e89]: Display name
                - textbox "Display name" [ref=e92]: Test Person
              - generic [ref=e93]:
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
              - generic [ref=e107]:
                - generic [ref=e108]: Current password
                - textbox "Current password" [ref=e111]
              - generic [ref=e112]:
                - generic [ref=e113]: New password
                - textbox "New password" [ref=e116]
                - generic [ref=e117]: At least 12 characters.
              - generic [ref=e118]:
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
     |                                                                   ^ Error: controls under 44 px: A.skip-link h=31 | A.shell__brand h=22 | BUTTON.shell__search h=32 | BUTTON. h=28 | BUTTON.shell__account h=38 | INPUT. h=23 | INPUT. h=23 | BUTTON. h=34 | INPUT. h=23 | INPUT. h=23 | INPUT. h=23 | BUTTON. h=34
  46 |     })
  47 |   }
  48 | })
  49 | 
```