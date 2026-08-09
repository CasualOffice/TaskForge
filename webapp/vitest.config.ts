/**
 * The test runner (docs/42 §Testing: Vitest + Testing Library).
 *
 * Separate from `vite.config.ts` on purpose: that file carries the bundle-report
 * plugin and the floor-ladder entry switch, and a test run that produced a
 * `bundle-report.json` would let the size gate pass against a measurement of the
 * test bundle.
 */
import react from '@vitejs/plugin-react'
import { defineConfig } from 'vitest/config'

export default defineConfig({
  plugins: [react()],
  test: {
    // jsdom, not a real browser: this layer has to run in CI in seconds with no
    // download. What it therefore cannot check is written down in
    // `src/a11y.test.tsx` rather than left for someone to assume.
    environment: 'jsdom',
    globals: false,
    include: ['src/**/*.test.{ts,tsx}'],
  },
})
