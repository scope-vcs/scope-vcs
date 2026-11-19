import assert from 'node:assert/strict'
import { readFile, stat } from 'node:fs/promises'
import { fileURLToPath, pathToFileURL } from 'node:url'
import { join } from 'node:path'
import { Worker } from 'node:worker_threads'
import { filesBelow, sumBytes } from './bundle-budget.mjs'

const MAX_BUNDLE_BYTES = 11 * 1024 * 1024
const MAX_BUNDLE_FILES = 440
const workerDirectory = fileURLToPath(new URL(
  '../.output/server/_workers',
  import.meta.url,
))
const workerEntry = join(
  workerDirectory,
  'code-render-worker.mjs',
)
const serverSsrDirectory = fileURLToPath(new URL(
  '../.output/server/_ssr',
  import.meta.url,
))

const workerFiles = await filesBelow(workerDirectory)
const bundleBytes = sumBytes(await Promise.all(
  workerFiles.map(async (path) => (await stat(path)).size),
))

assert.ok(workerFiles.includes(workerEntry), 'code render worker entry was not emitted')
assert.ok(
  bundleBytes <= MAX_BUNDLE_BYTES,
  `code render worker bundle is ${bundleBytes} bytes; budget is ${MAX_BUNDLE_BYTES}`,
)
assert.ok(
  workerFiles.length <= MAX_BUNDLE_FILES,
  `code render worker emitted ${workerFiles.length} files; budget is ${MAX_BUNDLE_FILES}`,
)

const ssrSources = await Promise.all(
  (await filesBelow(serverSsrDirectory))
    .filter((path) => path.endsWith('.mjs'))
    .map((path) => readFile(path, 'utf8')),
)
const serverSource = ssrSources.find((source) =>
  source.includes('scope.code-renderers-v1')
)
assert.ok(serverSource, 'code render server module was not emitted')
assert.match(
  serverSource,
  /\.\.\/_workers\/code-render-worker\.mjs/,
  'code render server module does not reference the emitted worker',
)
assert.doesNotMatch(
  serverSource,
  /code-render-worker\.ts/,
  'code render server module still references TypeScript source',
)

const workerBudget = {
  maxHunks: 64,
  maxOutputBytes: 256 * 1024,
  maxRenderedLines: 800,
}
assert.deepEqual(
  await runWorker({
    budget: workerBudget,
    kind: 'diff',
    newText: 'same\n',
    oldText: 'same\n',
    path: '/fixture.ts',
  }),
  { kind: 'empty' },
)
const rendered = await runWorker({
  budget: workerBudget,
  kind: 'diff',
  newText: 'export const value = 2\n',
  oldText: 'export const value = 1\n',
  path: '/fixture.ts',
})
assert.equal(rendered.kind, 'html')
assert.ok(Buffer.byteLength(rendered.html) <= workerBudget.maxOutputBytes)
const file = await runWorker({
  budget: { maxOutputBytes: workerBudget.maxOutputBytes },
  kind: 'file',
  path: '/Cargo.toml',
  text: '[package]\nname = "fixture"\n',
})
assert.equal(file.kind, 'html')
assert.ok(
  new Set(file.html.match(/--diffs-token-light:#\w+/g)).size > 1,
  'built code render worker did not highlight a TOML file',
)

console.log(
  `code render worker: ${workerFiles.length} files, ${bundleBytes} bytes, executable`,
)

function runWorker(workerData) {
  const worker = new Worker(pathToFileURL(workerEntry))
  return new Promise((resolve, reject) => {
    let settled = false
    const deadline = setTimeout(() => {
      void finish(new Error('built code render worker missed its smoke deadline'))
    }, 10_000)
    worker.once('message', (message) => {
      if (message.kind !== 'ready') {
        void finish(new Error('built code render worker did not announce readiness'))
        return
      }
      worker.once('message', (result) => void finish(null, result))
      worker.postMessage(workerData)
    })
    worker.once('error', (error) => void finish(error))
    worker.once('exit', (code) => {
      if (code !== 0) void finish(new Error(`built code render worker exited ${code}`))
    })

    async function finish(error, result) {
      if (settled) return
      settled = true
      clearTimeout(deadline)
      try {
        await worker.terminate()
      } catch {}
      if (error) reject(error)
      else resolve(result)
    }
  })
}
