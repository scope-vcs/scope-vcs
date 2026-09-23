import assert from 'node:assert/strict'
import { createRequire } from 'node:module'
import { mkdtemp, rm } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { fileURLToPath } from 'node:url'
import test from 'node:test'
import { chromium } from 'playwright'
import { createServer } from 'vite'

const require = createRequire(import.meta.url)

test('accepted summaries and reconnects reconcile the retained request queue without restarting the stream', async t => {
  const cacheDir = await mkdtemp(join(tmpdir(), 'scope-vite-refresh-'))
  t.after(() => rm(cacheDir, { recursive: true, force: true }))
  const server = await createServer({
    configFile: false, cacheDir,
    root: fileURLToPath(new URL('./fixtures/repo-refresh', import.meta.url)),
    server: { host: '127.0.0.1', port: 0, fs: { allow: [fileURLToPath(new URL('..', import.meta.url))] } },
    resolve: { alias: [
      { find: '@clerk/tanstack-react-start', replacement: fileURLToPath(new URL('./fixtures/repo-refresh/auth.ts', import.meta.url)) },
      { find: '@', replacement: fileURLToPath(new URL('../src', import.meta.url)) },
      ...['react/jsx-dev-runtime', 'react/jsx-runtime', 'react-dom/client', 'react'].map(name => ({ find: name, replacement: require.resolve(name) })),
    ] },
    oxc: { jsx: { runtime: 'automatic' } },
  })
  await server.listen()
  t.after(() => server.close())
  const browser = await chromium.launch({ headless: true })
  t.after(() => browser.close())
  const page = await browser.newPage()
  page.setDefaultTimeout(5_000)
  const errors = []
  page.on('pageerror', error => errors.push(error.message))
  await page.goto(server.resolvedUrls.local[0], { timeout: 30_000 })
  await page.waitForFunction(() => window.fixture?.server.queueReads >= 4)
  await page.evaluate(async () => { window.fixture.submit(['new-request']); await window.fixture.refresh() })
  await page.locator('[data-count]').filter({ hasText: '1' }).waitFor()
  await page.getByRole('listitem').filter({ hasText: 'new-request' }).waitFor()
  assert.equal(await page.evaluate(() => window.fixture.server.connections), 1)

  // Reuse the same loaded queue on a child-page round trip.
  const beforeNavigation = await page.evaluate(() => window.fixture.server.queueReads)
  await page.getByRole('button', { name: 'Navigate' }).click()
  await page.getByRole('button', { name: 'Navigate' }).click()
  await page.getByRole('listitem').filter({ hasText: 'new-request' }).waitFor()
  assert.equal(await page.evaluate(() => window.fixture.server.queueReads), beforeNavigation)

  // Neither count nor repository version changes, but membership does.
  await page.evaluate(async () => { window.fixture.submit(['replacement']); await window.fixture.refresh() })
  await page.getByRole('listitem').filter({ hasText: 'replacement' }).waitFor()
  assert.equal(await page.getByText('new-request', { exact: true }).count(), 0)
  assert.equal(await page.evaluate(() => window.fixture.server.connections), 1)

  // Periodic reconciliation refreshes the four sections once, after the
  // summary is accepted, without an earlier refresh being canceled/repeated.
  const beforeLag = await page.evaluate(() => window.fixture.server.queueReads)
  await page.evaluate(() => { window.fixture.submit(['after-lag']); window.fixture.lag() })
  await page.getByRole('listitem').filter({ hasText: 'after-lag' }).waitFor()
  assert.equal(await page.evaluate(() => window.fixture.server.queueReads), beforeLag + 4)

  const beforeVersion = await page.evaluate(() => window.fixture.server.queueReads)
  await page.evaluate(async () => { window.fixture.advanceVersion(); window.fixture.submit(['new-version']); await window.fixture.refresh() })
  await page.getByRole('listitem').filter({ hasText: 'new-version' }).waitFor()
  assert.equal(await page.evaluate(() => window.fixture.server.queueReads), beforeVersion + 4)

  await page.evaluate(() => window.fixture.interrupt())
  await page.waitForFunction(() => window.fixture.server.streams.size === 0)
  // Change committed during the reconnect delay, after the interruption refresh.
  await page.evaluate(() => window.fixture.submit(['during-disconnect']))
  await page.getByRole('listitem').filter({ hasText: 'during-disconnect' }).waitFor()
  assert.equal(await page.evaluate(() => window.fixture.server.connections), 2)
  const settled = await page.evaluate(() => ({ reads: window.fixture.server.summaryReads, connections: window.fixture.server.connections }))
  await page.waitForTimeout(150)
  assert.deepEqual(await page.evaluate(() => ({ reads: window.fixture.server.summaryReads, connections: window.fixture.server.connections })), settled)
  assert.deepEqual(errors, [])
})
