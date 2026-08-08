#!/usr/bin/env node
// Bundle-size gate for ADR-024. Reads dist/bundle-report.json (written by the
// `taskforge-bundle-report` plugin in vite.config.ts) and exits non-zero when
// the initial chunk exceeds its threshold.
//
// Deliberately a separate process from the build: CI can run this against a
// downloaded artifact, and the gate cannot be defeated by a build-time flag.
//
// Units are KiB (1024 bytes) everywhere, and the kB (1000-byte) equivalent is
// printed alongside. ADR-024 says "200 KB" without saying which; the two
// readings differ by 4.7 KiB, which is 2.3% of the budget — enough to decide a
// borderline PR, so the gate states its unit rather than assuming one.
//
// Usage:
//   node scripts/size-check.mjs [--report <path>] [--budget-kib <n>] [--metric gzip|brotli]

import { readFileSync } from 'node:fs'
import { resolve, dirname } from 'node:path'
import { fileURLToPath } from 'node:url'

const HERE = dirname(fileURLToPath(import.meta.url))

/** ADR-024 says 200; docs/42 says the number is provisional until measured.
 *  Overridable so a superseding ADR changes one value in CI, not this file. */
const DEFAULT_BUDGET_KIB = Number(process.env.TASKFORGE_BUNDLE_BUDGET_KIB ?? 200)
const DEFAULT_METRIC = process.env.TASKFORGE_BUNDLE_METRIC ?? 'gzip'

function parseArgs(argv) {
  const out = {
    report: resolve(HERE, '..', 'dist', 'bundle-report.json'),
    budgetKib: DEFAULT_BUDGET_KIB,
    metric: DEFAULT_METRIC,
  }
  for (let i = 0; i < argv.length; i += 1) {
    const flag = argv[i]
    const value = argv[i + 1]
    if (flag === '--report' && value !== undefined) {
      out.report = resolve(value)
      i += 1
    } else if (flag === '--budget-kib' && value !== undefined) {
      out.budgetKib = Number(value)
      i += 1
    } else if (flag === '--metric' && value !== undefined) {
      out.metric = value
      i += 1
    } else {
      console.error(`unknown or incomplete argument: ${flag}`)
      process.exit(2)
    }
  }
  if (!Number.isFinite(out.budgetKib) || out.budgetKib <= 0) {
    console.error(`invalid --budget-kib: ${out.budgetKib}`)
    process.exit(2)
  }
  if (out.metric !== 'gzip' && out.metric !== 'brotli') {
    console.error(`invalid --metric: ${out.metric} (expected gzip or brotli)`)
    process.exit(2)
  }
  return out
}

const kib = (bytes) => (bytes / 1024).toFixed(1)

const args = parseArgs(process.argv.slice(2))

let report
try {
  report = JSON.parse(readFileSync(args.report, 'utf8'))
} catch (err) {
  console.error(`cannot read bundle report at ${args.report}: ${err.message}`)
  console.error('run `pnpm build` first — the gate never passes on a missing measurement')
  process.exit(2)
}

const initial = report.initialTotal
const measured = initial[args.metric]
const budgetBytes = args.budgetKib * 1024

const line = (label, t, extra = '') =>
  console.log(`  ${label.padEnd(15)}${kib(t.gzip).padStart(7)} KiB gzip  ${kib(t.brotli).padStart(7)} KiB brotli${extra}`)

console.log(`TaskForge bundle gate — ADR-024 (gating metric: ${args.metric})`)
console.log(`  report:        ${args.report}`)
console.log(`  generated:     ${report.generatedAt}`)
console.log('')
line('initial JS:', report.initialJs.total, `   (${kib(report.initialJs.total.raw)} KiB raw, ${report.initialJs.files.length} file(s))`)
line('initial CSS:', report.initialCss.total, `   (${report.initialCss.files.length} file(s))`)
line('INITIAL TOTAL:', initial, `   = ${(initial[args.metric] / 1000).toFixed(1)} kB (1000-byte) ${args.metric}`)
line('lazy chunks:', report.lazy.total, `   (${report.lazy.files.length} file(s), NOT counted)`)
console.log('')
console.log('  initial files, largest first:')
for (const f of [...report.initialJs.files, ...report.initialCss.files]) {
  console.log(`    ${kib(f.gzip).padStart(7)} KiB gzip  ${kib(f.brotli).padStart(7)} KiB brotli  ${f.file}`)
}
console.log('')

if (measured > budgetBytes) {
  console.error(
    `FAIL: initial ${args.metric} ${kib(measured)} KiB exceeds the ${args.budgetKib} KiB budget by ${kib(measured - budgetBytes)} KiB.`,
  )
  console.error(
    'ADR-024: the budget is raised by a superseding ADR with the measurement attached, never by disabling this gate.',
  )
  process.exit(1)
}

console.log(
  `PASS: initial ${args.metric} ${kib(measured)} KiB is within the ${args.budgetKib} KiB budget (${kib(budgetBytes - measured)} KiB headroom).`,
)
