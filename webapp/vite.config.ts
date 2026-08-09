import { brotliCompressSync, constants as zlibConstants, gzipSync } from 'node:zlib'
import { resolve } from 'node:path'

import react from '@vitejs/plugin-react'
import { visualizer } from 'rollup-plugin-visualizer'
import { defineConfig, type Plugin, type Rollup } from 'vite'

type OutputBundle = Rollup.OutputBundle
type OutputChunk = Rollup.OutputChunk
type OutputAsset = Rollup.OutputAsset

/**
 * ADR-024 budgets the *initial* download of the authenticated shell, not the
 * whole `dist/`. "Initial" therefore means the entry chunk plus everything it
 * reaches through **static** imports; anything only reachable through a dynamic
 * import is a lazy chunk and is reported separately (docs/42 §What is not — lazy,
 * always).
 */
function initialChunkNames(bundle: OutputBundle): Set<string> {
  const chunks = new Map<string, OutputChunk>()
  for (const file of Object.values(bundle)) {
    if (file.type === 'chunk') chunks.set(file.fileName, file)
  }
  const initial = new Set<string>()
  const queue: string[] = []
  for (const chunk of chunks.values()) {
    if (chunk.isEntry) queue.push(chunk.fileName)
  }
  while (queue.length > 0) {
    const name = queue.pop()
    if (name === undefined || initial.has(name)) continue
    initial.add(name)
    // `imports` is static-only; `dynamicImports` is deliberately not followed.
    for (const dep of chunks.get(name)?.imports ?? []) queue.push(dep)
  }
  return initial
}

/** CSS reachable from the initial chunks blocks first paint, so it counts. */
function initialCssNames(bundle: OutputBundle, initial: Set<string>): Set<string> {
  const css = new Set<string>()
  for (const name of initial) {
    const file = bundle[name]
    if (file?.type !== 'chunk') continue
    const meta = (file as OutputChunk & { viteMetadata?: { importedCss?: Set<string> } }).viteMetadata
    for (const href of meta?.importedCss ?? []) css.add(href)
  }
  return css
}

interface SizeRow {
  file: string
  raw: number
  gzip: number
  brotli: number
}

function sizesOf(source: string | Uint8Array): Omit<SizeRow, 'file'> {
  const buf = typeof source === 'string' ? Buffer.from(source, 'utf8') : Buffer.from(source)
  return {
    raw: buf.byteLength,
    // Level 9 / quality 11: what a static host or CDN serves pre-compressed.
    // Reporting a weaker level would understate nothing but flatter nobody.
    gzip: gzipSync(buf, { level: 9 }).byteLength,
    brotli: brotliCompressSync(buf, {
      params: {
        [zlibConstants.BROTLI_PARAM_QUALITY]: 11,
        [zlibConstants.BROTLI_PARAM_SIZE_HINT]: buf.byteLength,
      },
    }).byteLength,
  }
}

function contentOf(file: OutputChunk | OutputAsset): string | Uint8Array {
  return file.type === 'chunk' ? file.code : file.source
}

/**
 * Emits `bundle-report.json` into the build output. `scripts/size-check.mjs` is
 * the CI gate that reads it; keeping measurement and enforcement in separate
 * processes means the gate can also run against a build it did not produce.
 */
function bundleReport(): Plugin {
  return {
    name: 'taskforge-bundle-report',
    apply: 'build',
    generateBundle(_options, bundle) {
      const initial = initialChunkNames(bundle)
      const css = initialCssNames(bundle, initial)
      const js: SizeRow[] = []
      const lazy: SizeRow[] = []
      const cssRows: SizeRow[] = []

      for (const file of Object.values(bundle)) {
        if (file.type === 'chunk') {
          const row = { file: file.fileName, ...sizesOf(contentOf(file)) }
          ;(initial.has(file.fileName) ? js : lazy).push(row)
        } else if (css.has(file.fileName)) {
          cssRows.push({ file: file.fileName, ...sizesOf(contentOf(file)) })
        }
      }

      const total = (rows: SizeRow[]): Omit<SizeRow, 'file'> =>
        rows.reduce(
          (acc, r) => ({ raw: acc.raw + r.raw, gzip: acc.gzip + r.gzip, brotli: acc.brotli + r.brotli }),
          { raw: 0, gzip: 0, brotli: 0 },
        )

      const report = {
        generatedAt: new Date().toISOString(),
        // Per-file sums, not a sum of one concatenated blob: HTTP compresses each
        // response separately, so this is the number that crosses the wire.
        initialJs: { files: js.sort((a, b) => b.gzip - a.gzip), total: total(js) },
        initialCss: { files: cssRows.sort((a, b) => b.gzip - a.gzip), total: total(cssRows) },
        initialTotal: total([...js, ...cssRows]),
        lazy: { files: lazy.sort((a, b) => b.gzip - a.gzip), total: total(lazy) },
      }

      // emitFile rather than writeFileSync: the output directory does not exist
      // yet at generateBundle time, and this keeps the report inside the artifact.
      this.emitFile({
        type: 'asset',
        fileName: 'bundle-report.json',
        source: `${JSON.stringify(report, null, 2)}\n`,
      })
    },
  }
}

// Set by scripts/measure-deps.mjs to build one dependency step in isolation.
const floorEntry = process.env['FLOOR_ENTRY']
const outDir = floorEntry === undefined ? 'dist' : `dist-floor/${floorEntry}`

/**
 * Where `pnpm dev` forwards `/api` to.
 *
 * `scripts/dev-up.sh` sets `VITE_API_URL` to the API it just started. The
 * browser still talks to the Vite origin, and Vite forwards — deliberately, not
 * for convenience: the session cookie is `HttpOnly` and `SameSite=Lax`, and the
 * API registers no CORS layer, so a client pointed straight at `:8080` from
 * `:5173` would have every authenticated request refused before it arrived. A
 * same-origin proxy is the only shape that works, and it is also the shape
 * production has (one origin, `/api` behind it).
 */
const apiTarget = process.env['VITE_API_URL'] ?? 'http://127.0.0.1:8080'

export default defineConfig({
  server: {
    proxy: {
      '/api': { target: apiTarget, changeOrigin: false },
    },
  },
  plugins: [
    react(),
    bundleReport(),
    visualizer({
      filename: resolve(import.meta.dirname, outDir, 'stats.html'),
      // gzip + brotli columns: docs/42 budgets compressed bytes, so an
      // uncompressed treemap would be the wrong thing to review a PR against.
      gzipSize: true,
      brotliSize: true,
      template: 'treemap',
    }),
  ],
  build: {
    outDir,
    emptyOutDir: true,
    // docs/18 §Browsers: last-2-major evergreen, ES2022 required. Targeting
    // lower would inflate the floor with transforms nobody in the matrix needs.
    target: 'es2022',
    sourcemap: false,
    reportCompressedSize: true,
    rollupOptions:
      floorEntry === undefined ? {} : { input: resolve(import.meta.dirname, `src/floor/${floorEntry}.tsx`) },
  },
})
