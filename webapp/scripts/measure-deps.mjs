#!/usr/bin/env node
// Per-dependency contribution for BUNDLE-FLOOR.md.
//
// Builds a ladder of entry points, each adding exactly one library to the one
// below it, and reports the *marginal* compressed cost of each step. Marginal,
// not standalone: shared code (React internals, TanStack's shared store) is
// attributed to whichever step pulls it in first, so the numbers sum to the
// floor but are order-dependent. That is stated in BUNDLE-FLOOR.md rather than
// papered over.

import { execFileSync } from 'node:child_process'
import { readFileSync, writeFileSync } from 'node:fs'
import { dirname, resolve } from 'node:path'
import { fileURLToPath } from 'node:url'

const HERE = dirname(fileURLToPath(import.meta.url))
const ROOT = resolve(HERE, '..')

const STEPS = [
  { entry: 'step-0-react', adds: 'react + react-dom' },
  { entry: 'step-1-query', adds: '@tanstack/react-query' },
  { entry: 'step-2-router', adds: '@tanstack/react-router' },
  { entry: 'step-3-virtual', adds: '@tanstack/react-virtual' },
  { entry: 'step-4-dndkit', adds: '@dnd-kit/core + @dnd-kit/sortable' },
]

const kib = (bytes) => (bytes / 1024).toFixed(1)

const rows = []
let prev = { gzip: 0, brotli: 0, raw: 0 }

for (const step of STEPS) {
  process.stderr.write(`building ${step.entry}…\n`)
  // Resolve the local binary rather than shelling out to a package manager, so
  // the ladder runs the same way under pnpm, npm, or CI.
  execFileSync(resolve(ROOT, 'node_modules', '.bin', 'vite'), ['build'], {
    cwd: ROOT,
    env: { ...process.env, FLOOR_ENTRY: step.entry },
    stdio: ['ignore', 'ignore', 'inherit'],
  })
  const report = JSON.parse(readFileSync(resolve(ROOT, 'dist-floor', step.entry, 'bundle-report.json'), 'utf8'))
  const total = report.initialTotal
  rows.push({
    entry: step.entry,
    adds: step.adds,
    cumulative: total,
    marginal: { raw: total.raw - prev.raw, gzip: total.gzip - prev.gzip, brotli: total.brotli - prev.brotli },
  })
  prev = total
}

console.log('\nAll sizes in KiB (1024 bytes), initial chunk only.\n')
console.log('| Step adds | marginal gzip | marginal brotli | cumulative gzip | cumulative brotli |')
console.log('| --- | ---: | ---: | ---: | ---: |')
for (const r of rows) {
  console.log(`| ${r.adds} | ${kib(r.marginal.gzip)} KiB | ${kib(r.marginal.brotli)} KiB | ${kib(r.cumulative.gzip)} KiB | ${kib(r.cumulative.brotli)} KiB |`)
}

writeFileSync(resolve(ROOT, 'dist-floor', 'ladder.json'), `${JSON.stringify(rows, null, 2)}\n`)
console.log(`\nwrote ${resolve(ROOT, 'dist-floor', 'ladder.json')}`)
