/**
 * The lint gate (C-019).
 *
 * # What this exists to catch, and what it does not
 *
 * docs/42 §Accessibility sets WCAG 2.2 AA and says plainly that "automation
 * catches perhaps a third of real issues". This config is the cheapest and
 * earliest third: `jsx-a11y` reads the source, so it fails a pull request in
 * seconds without a browser, a server, or a database — which is what makes it
 * runnable in `scripts/check.sh` beside the bundle gate.
 *
 * It catches the classes that are invisible in review and obvious to a screen
 * reader: a control with no accessible name, a click handler on a `<div>`, an
 * `aria-*` attribute that does not exist, a label pointing at nothing.
 *
 * It does **not** catch contrast, focus order, live-region behaviour, or
 * anything that depends on rendered output. Those need the axe run in
 * `src/a11y.test.tsx` and the manual keyboard pass docs/42 requires per
 * release. Three layers, each honest about its reach; none of them is called
 * "the accessibility gate" on its own.
 *
 * # Why the a11y rules are errors and the style rules are absent
 *
 * A warning is a thing CI prints and nobody reads. Every rule here fails the
 * build or is not here. There is deliberately no formatting or stylistic rule
 * set: those produce noise that trains contributors to run `--fix` without
 * looking, which is exactly the habit that lets a real a11y error through.
 */
import js from '@eslint/js'
import jsxA11y from 'eslint-plugin-jsx-a11y'
import reactHooks from 'eslint-plugin-react-hooks'
import tseslint from 'typescript-eslint'

export default tseslint.config(
  {
    // The floor harness measures dependencies, not product code, and its
    // components are deliberately minimal — linting them would produce
    // failures whose only fix is to make the measurement wrong.
    ignores: ['dist/**', 'dist-floor/**', 'src/floor/**', 'node_modules/**'],
  },
  js.configs.recommended,
  ...tseslint.configs.recommended,
  {
    // The build scripts and the Vite config run in Node, not a browser. Given
    // their own block rather than adding `process` to the browser globals: a
    // `process.env` reference in a component is a build that breaks in a tab,
    // and the lint should say so there.
    files: ['scripts/**/*.mjs', 'vite.config.ts', 'eslint.config.js'],
    languageOptions: {
      globals: { process: 'readonly', console: 'readonly', Buffer: 'readonly' },
    },
  },
  {
    files: ['src/**/*.{ts,tsx}'],
    plugins: { 'jsx-a11y': jsxA11y, 'react-hooks': reactHooks },
    languageOptions: {
      parserOptions: { ecmaFeatures: { jsx: true } },
      globals: {
        window: 'readonly',
        document: 'readonly',
        localStorage: 'readonly',
        crypto: 'readonly',
        fetch: 'readonly',
        Headers: 'readonly',
        Response: 'readonly',
        Request: 'readonly',
        AbortSignal: 'readonly',
        TextEncoder: 'readonly',
        setTimeout: 'readonly',
        clearTimeout: 'readonly',
        requestAnimationFrame: 'readonly',
        console: 'readonly',
        HTMLElement: 'readonly',
        HTMLDivElement: 'readonly',
        KeyboardEvent: 'readonly',
        URLSearchParams: 'readonly',
      },
    },
    rules: {
      // ── Accessibility (docs/42 §Accessibility) ─────────────────────────
      ...jsxA11y.flatConfigs.recommended.rules,
      // Raised from the recommended set. Both are the difference between a
      // control a keyboard can reach and one it cannot, which is the single
      // most common way a React interface excludes people.
      'jsx-a11y/no-static-element-interactions': 'error',
      'jsx-a11y/click-events-have-key-events': 'error',
      'jsx-a11y/no-autofocus': 'error',

      // ── Hooks ──────────────────────────────────────────────────────────
      // A missing dependency is how an optimistic update reads a stale task and
      // sends the wrong `If-Match`. Not a style rule.
      'react-hooks/rules-of-hooks': 'error',
      'react-hooks/exhaustive-deps': 'error',

      // ── Correctness ────────────────────────────────────────────────────
      // `strict` already forbids implicit any; this catches the explicit escape
      // hatch, which is where a wire type stops being checked against docs/05.
      '@typescript-eslint/no-explicit-any': 'error',
      '@typescript-eslint/no-unused-vars': [
        'error',
        { argsIgnorePattern: '^_', varsIgnorePattern: '^_' },
      ],
    },
  },
  {
    // The scrim is a click target with a keyboard equivalent (`Escape`, wired by
    // the focus trap) and is `aria-hidden`, which is the documented pattern for
    // a modal backdrop. The rule cannot see the Escape handler, so it is turned
    // off HERE — for two files, named — rather than globally.
    files: ['src/drawer/TaskDrawer.tsx', 'src/palette/CommandPalette.tsx'],
    rules: {
      'jsx-a11y/no-static-element-interactions': 'off',
      'jsx-a11y/click-events-have-key-events': 'off',
    },
  },
)
