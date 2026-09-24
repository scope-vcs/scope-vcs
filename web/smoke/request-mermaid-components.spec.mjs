import assert from 'node:assert/strict'
import { createRequire } from 'node:module'
import { mkdtemp, rm } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { fileURLToPath } from 'node:url'
import test from 'node:test'
import { gzipSync } from 'node:zlib'
import { chromium } from 'playwright'
import { build, preview } from 'vite'
import tailwindcss from '@tailwindcss/vite'

const require = createRequire(import.meta.url)
// RAIL: past 1,000 ms a user loses focus on the task.
const MAX_COLD_DIAGRAM_MS = 1000
// Measured 212 KiB plus headroom; one diagram-bearing request stays under the initial route budget.
const MAX_LAZY_DIAGRAM_GZIP_BYTES = 256 * 1024

test('request diagrams load on demand and reuse rendered output across navigation', { timeout: 180_000 }, async (t) => {
  const directory = await mkdtemp(join(tmpdir(), 'scope-mermaid-'))
  t.after(() => rm(directory, { recursive: true, force: true }))
  const config = {
    configFile: false,
    cacheDir: join(directory, 'cache'),
    root: fileURLToPath(new URL('./fixtures/request-mermaid', import.meta.url)),
    plugins: [tailwindcss()],
    resolve: { alias: [
      { find: '@clerk/tanstack-react-start', replacement: fileURLToPath(new URL('./fixtures/request-mermaid/clerk.ts', import.meta.url)) },
      { find: '@', replacement: fileURLToPath(new URL('../src', import.meta.url)) },
      ...['react/jsx-dev-runtime', 'react/jsx-runtime', 'react-dom/client', 'react']
        .map((name) => ({ find: name, replacement: require.resolve(name) })),
    ] },
    oxc: { jsx: { runtime: 'automatic' } },
    build: { outDir: join(directory, 'dist'), emptyOutDir: true },
    logLevel: 'warn',
  }
  const output = await build(config)
  const chunks = output.output.filter((item) => item.type === 'chunk')
  const diagramChunks = new Set(chunks.filter((chunk) => Object.keys(chunk.modules)
    .some((id) => id.includes('/node_modules/mermaid/'))).map((chunk) => chunk.fileName))
  assert(diagramChunks.size > 0)
  const initial = new Set()
  const visit = (chunk) => {
    if (initial.has(chunk.fileName)) return
    initial.add(chunk.fileName)
    for (const name of chunk.imports) {
      const imported = chunks.find((candidate) => candidate.fileName === name)
      if (imported) visit(imported)
    }
  }
  chunks.filter((chunk) => chunk.isEntry).forEach(visit)
  assert.deepEqual([...initial].filter((name) => diagramChunks.has(name)), [], 'Mermaid must remain outside static entry imports')

  const server = await preview({ ...config, preview: { host: '127.0.0.1', port: 0 } })
  t.after(() => new Promise((resolve) => server.httpServer.close(resolve)))
  const browser = await chromium.launch({ headless: true })
  t.after(() => browser.close())
  const page = await browser.newPage({ viewport: { width: 1280, height: 900 }, colorScheme: 'light' })
  page.setDefaultTimeout(30_000)
  if (process.env.SCOPE_MERMAID_CPU_RATE) {
    const session = await page.context().newCDPSession(page)
    await session.send('Emulation.setCPUThrottlingRate', { rate: Number(process.env.SCOPE_MERMAID_CPU_RATE) })
  }
  const errors = []
  const fetched = []
  page.on('pageerror', (error) => errors.push(error.message))
  page.on('request', (request) => fetched.push(request.url()))
  await page.addInitScript(() => {
    window.diagramLongTasks = []
    new PerformanceObserver((list) => window.diagramLongTasks.push(...list.getEntries().map((entry) => entry.duration)))
      .observe({ type: 'longtask', buffered: true })
    window.liveDiagramUrls = new Set()
    const create = URL.createObjectURL.bind(URL)
    const revoke = URL.revokeObjectURL.bind(URL)
    URL.createObjectURL = (blob) => { const url = create(blob); window.liveDiagramUrls.add(url); return url }
    URL.revokeObjectURL = (url) => { window.liveDiagramUrls.delete(url); revoke(url) }
  })
  await page.goto(server.resolvedUrls.local[0])
  await page.getByText('const ready = true', { exact: true }).waitFor()
  assert.equal(fetched.some((url) => [...diagramChunks].some((name) => url.endsWith(name))), false)

  const started = await page.evaluate(() => performance.now())
  await page.getByRole('button', { name: 'Show diagrams', exact: true }).click()
  const description = page.getByRole('region', { name: 'Description', exact: true })
  const discussion = page.getByRole('region', { name: 'Discussion', exact: true })
  await loadedImage(description)
  await loadedImage(discussion)
  const firstDiagramMs = await page.evaluate((start) => performance.now() - start, started)
  const lazyBytes = chunks.filter((chunk) => !initial.has(chunk.fileName) && fetched.some((url) => url.endsWith(chunk.fileName)))
    .reduce((bytes, chunk) => bytes + gzipSync(chunk.code).byteLength, 0)
  assert.ok(firstDiagramMs <= MAX_COLD_DIAGRAM_MS,
    `cold diagram render took ${Math.round(firstDiagramMs)} ms, exceeding the ${MAX_COLD_DIAGRAM_MS} ms RAIL focus-loss boundary`)
  assert.ok(lazyBytes <= MAX_LAZY_DIAGRAM_GZIP_BYTES,
    `diagram lazy JS is ${Math.round(lazyBytes / 1024)} KiB gzip, over the ${MAX_LAZY_DIAGRAM_GZIP_BYTES / 1024} KiB cap`)
  t.diagnostic(`Longest task during cold diagram render: ${Math.round(await page.evaluate(() => Math.max(0, ...window.diagramLongTasks)))}ms.`)
  assert.equal(await page.getByRole('region', { name: 'Reply', exact: true }).locator('img').count(), 0)
  assert.equal((await page.evaluate(() => window.mermaidCacheStats())).entries, 1)
  const originalSvg = await svgText(description)
  assert.match(originalSvg, /Request[\s\S]*opened/)
  assert.match(originalSvg, /Review[\s\S]*changes/)
  assert.equal(await description.locator('pre > [data-mermaid-block]').count(), 0)
  await description.getByText('View source', { exact: true }).click()
  await description.locator('details pre').waitFor({ state: 'visible' })
  assert.match(await description.locator('details pre').textContent(), /flowchart LR/)

  await page.getByRole('button', { name: 'Edit prose', exact: true }).click()
  await page.getByText('Updated explanation.', { exact: true }).waitFor()
  assert.equal(await svgText(description), originalSvg)
  await page.getByRole('button', { name: 'Toggle request', exact: true }).click()
  await page.getByText('Request closed', { exact: true }).waitFor()
  assert.equal(await page.evaluate(() => window.liveDiagramUrls.size), 0)
  await page.getByRole('button', { name: 'Toggle request', exact: true }).click()
  await loadedImage(description)
  assert.equal(await svgText(description), originalSvg, 'reopening reuses identical SVG, including generated IDs')

  await page.getByRole('button', { name: 'Toggle theme', exact: true }).click()
  await page.waitForFunction(() => window.mermaidCacheStats().entries === 2)
  await loadedImage(description)
  assert.notEqual(await svgText(description), originalSvg)
  if (process.env.SCOPE_COMPONENT_SCREENSHOT) await page.screenshot({ path: `${process.env.SCOPE_COMPONENT_SCREENSHOT}.dark.png` })
  await page.getByRole('button', { name: 'Toggle theme', exact: true }).click()
  await loadedImage(description)
  assert.equal(await svgText(description), originalSvg)

  await page.setViewportSize({ width: 375, height: 844 })
  assert.equal(await page.evaluate(() => document.documentElement.scrollWidth > innerWidth), false)
  if (process.env.SCOPE_COMPONENT_SCREENSHOT) await page.screenshot({ path: process.env.SCOPE_COMPONENT_SCREENSHOT })
  const reply = page.getByRole('region', { name: 'Reply', exact: true })
  await reply.evaluate((element) => {
    const root = document.getElementById('main-content')
    root.scrollTop += element.getBoundingClientRect().top - root.getBoundingClientRect().bottom - 150
  })
  await loadedImage(reply)
  assert(await reply.evaluate((element) => element.getBoundingClientRect().top >
    document.getElementById('main-content').getBoundingClientRect().bottom), 'nearby reply renders before entering the scroll container viewport')
  assert.match(await svgText(reply), /Review[\s\S]*request/)
  assert.equal((await page.evaluate(() => window.mermaidCacheStats())).entries, 3)

  await page.getByRole('button', { name: 'Other diagrams', exact: true }).click()
  await loadedImage(description)
  await loadedImage(discussion)
  assert.match(await svgText(description), /Draft/)
  assert.match(await svgText(discussion), /REQUEST/)
  const oldUrl = await description.locator('img').getAttribute('src')
  await page.evaluate(() => window.changeMermaidViewer())
  await description.locator('[data-mermaid-block]').waitFor({ state: 'detached' })
  assert.equal(await page.evaluate(() => window.liveDiagramUrls.size), 0, 'stale viewer diagrams release their image URLs')
  await page.evaluate(() => window.refreshMermaidViewer())
  await page.waitForFunction((url) => {
    const image = document.querySelector('[aria-label="Description"] img')
    return image && image.src !== url
  }, oldUrl)
  await loadedImage(description)
  assert.match(await svgText(description), /Draft/)

  await page.getByRole('button', { name: 'Invalid diagram', exact: true }).click()
  await description.getByRole('status').waitFor()
  await description.locator('details pre').waitFor({ state: 'visible' })
  assert.equal(await description.locator('img').count(), 0)
  await page.getByRole('button', { name: 'Unsafe diagram', exact: true }).click()
  await description.getByRole('status').waitFor()
  assert.equal(fetched.some((url) => url.includes('example.invalid')), false)
  assert.deepEqual(errors, [])
})

async function loadedImage(region) {
  await region.locator('img').waitFor()
  await region.locator('img').evaluate((image) => image.decode())
}
async function svgText(region) {
  return region.locator('img').evaluate(async (image) => (await fetch(image.src)).text())
}
