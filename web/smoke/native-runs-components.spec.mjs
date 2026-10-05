import assert from 'node:assert/strict'
import { createRequire } from 'node:module'
import { mkdtemp, rm } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { fileURLToPath } from 'node:url'
import test from 'node:test'
import { chromium } from 'playwright'
import { createServer } from 'vite'
import tailwindcss from '@tailwindcss/vite'

const require = createRequire(import.meta.url)
test('seeded native run navigation makes one read per resource on cold load and reopen', async (t) => {
  const cacheDir = await mkdtemp(join(tmpdir(), 'scope-vite-native-runs-'))
  t.after(() => rm(cacheDir, { recursive: true, force: true }))
  const server = await createServer({
    cacheDir,
    configFile: false, root: fileURLToPath(new URL('./fixtures/native-runs', import.meta.url)),
    plugins: [tailwindcss()], server: { host: '0.0.0.0', port: 0, fs: { allow: [fileURLToPath(new URL('..', import.meta.url))] } },
    resolve: { alias: [
      { find: '@clerk/tanstack-react-start', replacement: fileURLToPath(new URL('./fixtures/request-workspace/clerk.tsx', import.meta.url)) },
      { find: '@/routes/-run-history-actions', replacement: fileURLToPath(new URL('./fixtures/native-runs/actions.ts', import.meta.url)) },
      { find: '@/routes/-repo-settings-actions', replacement: fileURLToPath(new URL('./fixtures/native-runs/actions.ts', import.meta.url)) },
      { find: '@', replacement: fileURLToPath(new URL('../src', import.meta.url)) },
      ...['react/jsx-dev-runtime', 'react/jsx-runtime', 'react-dom/client', 'react'].map((name) => ({ find: name, replacement: require.resolve(name) })),
    ] }, oxc: { jsx: { runtime: 'automatic' } },
  })
  t.after(() => server.close())
  await server.listen()
  const browser = await chromium.launch({ headless: true })
  t.after(() => browser.close())
  const page = await browser.newPage({ viewport: { width: 1280, height: 900 } })
  const errors = []
  page.on('pageerror', (error) => errors.push(error.message))
  await page.goto(new URL('/owner/repo/runs', server.resolvedUrls.local[0]).href)
  await page.getByRole('heading', { name: 'Runs', exact: true }).waitFor()
  await page.evaluate(() => new Promise((resolve) => setTimeout(resolve, 100)))
  assert.equal(await page.evaluate(() => window.loads.history), 1, 'seeded cold history load must not read again on mount')
  await page.getByRole('link', { name: 'Detail', exact: true }).click()
  await page.getByRole('heading', { name: /Runs.*tests/ }).waitFor()
  await page.evaluate(() => new Promise((resolve) => setTimeout(resolve, 100)))
  assert.equal(await page.evaluate(() => window.loads.detail), 1, 'seeded cold detail load must not read again on mount')
  await page.getByRole('link', { name: 'History', exact: true }).click()
  await page.getByRole('heading', { name: 'Runs', exact: true }).waitFor()
  await page.getByRole('link', { name: 'Detail', exact: true }).click()
  await page.getByRole('heading', { name: /Runs.*tests/ }).waitFor()
  assert.deepEqual(await page.evaluate(() => ({ history: window.loads.history, detail: window.loads.detail })), { history: 1, detail: 1 })
  for (const width of [1280, 390]) {
    await page.setViewportSize({ width, height: 844 })
    assert.equal(await page.evaluate(() => document.documentElement.scrollWidth <= window.innerWidth), true)
    if (process.env.SCOPE_COMPONENT_SCREENSHOT) await page.screenshot({ animations: 'disabled', path: `${process.env.SCOPE_COMPONENT_SCREENSHOT}-${width}.png` })
  }
  await page.evaluate(() => window.emitChange('Connected'))
  assert.deepEqual(await page.evaluate(() => ({ history: window.loads.history, detail: window.loads.detail })), { history: 1, detail: 1 })
  await page.evaluate(() => window.emitChange('Lagged'))
  await page.waitForFunction(() => window.loads.detail === 2)
  // The live event owner refreshes native metadata while retaining the view.
  await page.evaluate(() => { window.holdLoads(); window.completeRun(); window.emitChange({ RunChanged: { run_id: 'run-1', change: 'StatusChanged' } }) })
  await page.waitForFunction(() => window.loads.detail === 3)
  assert.equal(await page.getByRole('heading', { name: /Runs.*tests/ }).count(), 1)
  await page.evaluate(() => window.releaseLoads())
  await page.getByRole('button', { name: 'Run again' }).waitFor()
  await page.getByRole('link', { name: 'History', exact: true }).click()
  await page.getByRole('heading', { name: 'Runs', exact: true }).waitFor()
  await page.getByRole('button', { name: 'Load older runs' }).click()
  await page.getByText('older tests', { exact: true }).waitFor()
  await page.getByRole('link', { name: 'Away', exact: true }).click()
  await page.evaluate(() => window.emitChange({ RunChanged: { run_id: 'run-1', change: 'Created' } }))
  await page.evaluate(() => window.holdLoads())
  await page.getByRole('link', { name: 'History', exact: true }).click()
  await page.getByText('older tests', { exact: true }).waitFor()
  await page.evaluate(() => window.releaseLoads())
  await page.getByRole('link', { name: 'Detail', exact: true }).click()
  await page.getByText('retained build output', { exact: false }).waitFor()
  const completedReads = await page.evaluate(() => window.loads.detail)
  await page.getByRole('link', { name: 'History', exact: true }).click()
  await page.getByRole('link', { name: 'Detail', exact: true }).click()
  await page.getByText('retained build output', { exact: false }).waitFor()
  assert.equal(await page.evaluate(() => window.loads.detail), completedReads)
  // A different access identity gets no old SSR handoff or retained detail/logs.
  await page.evaluate(() => { window.holdLoads(); window.setPermitted(false); window.setActor('Public') })
  await page.getByText('Loading run details', { exact: true }).waitFor()
  assert.equal(await page.getByText('retained build output', { exact: false }).count(), 0)
  assert.equal(await page.getByRole('heading', { name: /Runs.*tests/ }).count(), 0)
  await page.evaluate(() => window.releaseLoads())
  await page.getByText('Run denied', { exact: true }).waitFor()
  // A new viewer also cannot reuse the old scope's metadata or log handoff.
  await page.evaluate(() => { window.holdLoads(); window.setViewer('other') })
  await page.getByText('Loading run details', { exact: true }).waitFor()
  assert.equal(await page.getByText('retained build output', { exact: false }).count(), 0)
  await page.evaluate(() => window.releaseLoads())
  await page.getByText('Run denied', { exact: true }).waitFor()

  // Unseeded client navigation reads each resource once; cancellation refreshes detail.
  const cold = await browser.newPage()
  cold.on('pageerror', (error) => errors.push(error.message))
  await cold.goto(new URL('/owner/repo/runs?client=true', server.resolvedUrls.local[0]).href)
  await cold.getByRole('heading', { name: 'Runs', exact: true }).waitFor()
  assert.equal(await cold.evaluate(() => window.loads.history), 1)
  await cold.getByRole('link', { name: 'Detail', exact: true }).click()
  await cold.getByRole('heading', { name: /Runs.*tests/ }).waitFor()
  assert.equal(await cold.evaluate(() => window.loads.detail), 1)
  await cold.getByRole('button', { name: 'Cancel', exact: true }).click()
  await cold.getByRole('button', { name: 'Run again' }).waitFor()
  assert.equal(await cold.evaluate(() => window.loads.detail), 2)
  assert.deepEqual(errors, [])
})
