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

test('a refresh that finds the discussion retained keeps the open composer', async (t) => {
  const cacheDir = await mkdtemp(join(tmpdir(), 'scope-vite-request-discussion-page-'))
  t.after(() => rm(cacheDir, { recursive: true, force: true }))
  const server = await createServer({
    cacheDir,
    configFile: false,
    root: fileURLToPath(new URL('./fixtures/request-discussion-page', import.meta.url)),
    server: {
      host: '127.0.0.1',
      port: 0,
      fs: { allow: [fileURLToPath(new URL('..', import.meta.url))] },
    },
    resolve: { alias: [
      { find: '@', replacement: fileURLToPath(new URL('../src', import.meta.url)) },
      ...['react/jsx-dev-runtime', 'react/jsx-runtime', 'react-dom/client', 'react']
        .map((name) => ({ find: name, replacement: require.resolve(name) })),
    ] },
    oxc: { jsx: { runtime: 'automatic' } },
  })
  await server.listen()
  t.after(() => server.close())
  const browser = await chromium.launch({ headless: true })
  t.after(() => browser.close())
  const page = await browser.newPage()
  page.setDefaultTimeout(10_000)
  const errors = []
  page.on('pageerror', (error) => errors.push(error.message))
  await page.goto(server.resolvedUrls.local[0], { timeout: 30_000, waitUntil: 'domcontentloaded' })

  await page.getByRole('status').filter({ hasText: 'Loading request discussion' }).waitFor({ state: 'attached' })
  await page.evaluate(() => window.resolveFirstLoad())
  const composer = page.getByRole('textbox', { name: 'Start a new discussion' })
  await composer.fill('Why this file?')
  await composer.evaluate((element) => { window.openComposer = element })

  await page.evaluate(() => window.refreshFromRetained())
  await page.evaluate(() => new Promise((resolve) => requestAnimationFrame(() => requestAnimationFrame(resolve))))
  assert.equal(await composer.inputValue(), 'Why this file?')
  assert.equal(await composer.evaluate((element) => element === window.openComposer), true)
  assert.deepEqual(errors, [])
})
