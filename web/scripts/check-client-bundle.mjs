import assert from 'node:assert/strict'
import { readFile } from 'node:fs/promises'
import { relative } from 'node:path'
import { fileURLToPath } from 'node:url'
import { gzipSync } from 'node:zlib'
import { filesBelow, sumBytes } from './bundle-budget.mjs'

// Lighthouse's example script budget, under the 307 KiB JavaScript budget in
// "The Performance Inequality Gap, 2026" (infrequently.org). Measured 265 KiB.
const MAX_INITIAL_GZIP_BYTES = 300 * 1024
// Roughly one second of transfer on a 4 Mbps link.
const MAX_CHUNK_GZIP_BYTES = 512 * 1024

const publicDirectory = fileURLToPath(new URL('../.output/public', import.meta.url))
const chunkGraphPath = fileURLToPath(new URL('../.output/client-chunk-graph.json', import.meta.url))

const chunkGraph = JSON.parse(await readFile(chunkGraphPath, 'utf8'))
const chunksByName = new Map(chunkGraph.map((chunk) => [chunk.fileName, chunk]))
const scripts = await Promise.all((await filesBelow(publicDirectory))
  .filter((path) => path.endsWith('.js'))
  .map(async (path) => ({
    name: relative(publicDirectory, path),
    gzipBytes: gzipSync(await readFile(path)).byteLength,
  })))
scripts.sort((left, right) => right.gzipBytes - left.gzipBytes)
const gzipBytesByName = new Map(scripts.map((script) => [script.name, script.gzipBytes]))

for (const name of chunksByName.keys()) {
  assert.ok(gzipBytesByName.has(name), `client chunk graph names ${name}, which was not emitted`)
}

const initial = new Set()
const visit = (name) => {
  if (initial.has(name)) return
  const chunk = chunksByName.get(name)
  assert.ok(chunk, `client chunk graph imports ${name}, which it does not describe`)
  initial.add(name)
  chunk.imports.forEach(visit)
}
chunkGraph.filter((chunk) => chunk.isEntry).forEach((chunk) => visit(chunk.fileName))
assert.ok(initial.size > 0, 'client chunk graph has no entry chunk')

const initialScripts = scripts.filter((script) => initial.has(script.name))
const initialGzipBytes = sumBytes(initialScripts.map((script) => script.gzipBytes))
const failures = []
if (initialGzipBytes > MAX_INITIAL_GZIP_BYTES) {
  failures.push(
    `initial client JS is ${initialGzipBytes} bytes gzip, ${initialGzipBytes - MAX_INITIAL_GZIP_BYTES} over the ${MAX_INITIAL_GZIP_BYTES} byte budget:`,
    ...initialScripts.map((script) => `  ${script.name} ${script.gzipBytes} bytes gzip`),
  )
}
for (const script of scripts.filter((script) => script.gzipBytes > MAX_CHUNK_GZIP_BYTES)) {
  failures.push(
    `client chunk ${script.name} is ${script.gzipBytes} bytes gzip, ${script.gzipBytes - MAX_CHUNK_GZIP_BYTES} over the ${MAX_CHUNK_GZIP_BYTES} byte per-chunk budget`,
  )
}
if (failures.length > 0) {
  console.error(failures.join('\n'))
  process.exit(1)
}

const [largest] = scripts
console.log(
  `client bundle: initial ${initialGzipBytes} bytes gzip; largest ${largest.name} ${largest.gzipBytes} bytes gzip; total ${sumBytes(scripts.map((script) => script.gzipBytes))} bytes gzip across ${scripts.length} chunks`,
)
