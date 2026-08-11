/**
 * The browser layer (`docs/47` §9).
 *
 * # What only a browser can check
 *
 * jsdom has no layout and no painting, so the suite that calls itself the
 * accessibility gate cannot see contrast, focus order, or whether an element is
 * on the screen at all. Three consecutive changes shipped with "not visually
 * verified" in their descriptions — a popover positioned off-screen, a metadata
 * rail stacked above the title it belonged under, a list row whose title
 * collapsed to nothing — and every one of them was a geometry bug that every
 * other gate passed.
 *
 * These tests assert geometry: what is on the screen, how wide it is, whether
 * anything overflows, and how big a control is under a finger.
 *
 * # Why it serves the built app with a stubbed API
 *
 * The alternative is a running database, a running worker and seeded data, which
 * makes a layout test fail for reasons that have nothing to do with layout. The
 * server's behaviour is covered by the Rust suites; what is unproven is what the
 * client does with a response, so the response is a fixture and the browser is
 * real.
 */
import { defineConfig, devices } from '@playwright/test'

const PORT = 4173

export default defineConfig({
  testDir: './e2e',
  // Geometry is deterministic; a retry that passes is a flake being hidden.
  retries: 0,
  fullyParallel: true,
  reporter: process.env.CI === undefined ? 'list' : 'github',
  use: {
    baseURL: `http://127.0.0.1:${PORT}`,
    trace: 'retain-on-failure',
  },
  projects: [
    { name: 'desktop', use: { ...devices['Desktop Chrome'] } },
    {
      name: 'phone',
      // A real phone profile: 390 px wide, coarse pointer, device pixel ratio
      // 3. `hasTouch` is what makes the 44 px tier apply.
      use: { ...devices['iPhone 13'] },
    },
  ],
  webServer: {
    // `--host 127.0.0.1`: preview binds IPv6 `localhost` by default, and the
    // base URL is v4, so without this every request is refused.
    command: `pnpm vite preview --host 127.0.0.1 --port ${PORT} --strictPort`,
    port: PORT,
    reuseExistingServer: process.env.CI === undefined,
    timeout: 120_000,
  },
})
