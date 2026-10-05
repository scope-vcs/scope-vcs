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

test('request checks lead with what is left, fold the rest, and only link Scope runs', async (t) => {
  const cacheDir = await mkdtemp(join(tmpdir(), 'scope-vite-request-checks-'))
  t.after(() => rm(cacheDir, { recursive: true, force: true }))
  const server = await createServer({
    cacheDir,
    configFile: false,
    root: fileURLToPath(new URL('./fixtures/request-checks', import.meta.url)),
    plugins: [tailwindcss()],
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
  const browser = await chromium.launch({ headless: true })
  const page = await browser.newPage({ viewport: { width: 1280, height: 900 } })
  page.setDefaultTimeout(10_000)
  try {
    const errors = []
    page.on('pageerror', (error) => errors.push(error.message))
    await page.goto(new URL('octo/demo/requests/req_1', server.resolvedUrls.local[0]).href, {
      timeout: 30_000,
      waitUntil: 'domcontentloaded',
    })
    const checks = page.getByRole('region', { name: 'Checks' })

    await checks.getByText('3 of 8 left', { exact: true }).waitFor()
    await checks.getByText('· 5 passed · 4 skipped', { exact: true }).waitFor()
    await checks.getByText('Checks run publicly, so this private request’s changes are public.', { exact: true }).waitFor()
    assert.equal(await checks.getByText(/GitHub|scope\/requests/).count(), 0)
    assert.deepEqual(
      await checks.getByRole('listitem').evaluateAll((items) => items.map((item) => item.textContent)),
      [
        'Check operationsin progress',
        'Validate selected components\u00a0/\u00a0CLI validationwaiting',
        'Validate selected components\u00a0/\u00a0Integration validationwaiting',
      ],
    )

    await checks.getByRole('button', { name: 'Show all 12' }).click()
    await checks.getByRole('listitem').filter({ hasText: /^Server validation$/ }).waitFor()
    assert.equal(await checks.getByRole('listitem').count(), 14)
    await page.evaluate(() => window.refresh())
    await checks.getByText('2 of 8 left', { exact: true }).waitFor()
    assert.equal(await checks.getByRole('button', { name: 'Show less' }).getAttribute('aria-expanded'), 'true')

    await checks.getByRole('button', { name: 'Show less' }).click()
    await page.setViewportSize({ width: 390, height: 844 })
    assert.equal(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth), true)
    const job = checks.getByText('Integration validation', { exact: true })
    assert.equal(await job.evaluate((element) => element.scrollWidth <= element.clientWidth), true)
    assert.equal(await checks.getByText(/^Validate selected components/).first().isVisible(), true)
    await checks.getByRole('button', { name: 'Show all 12' }).click()

    await page.evaluate(() => window.showNative())
    await checks.getByText('1 failed', { exact: true }).waitFor()
    assert.equal(await checks.locator('a[href^="http"]').count(), 0)
    await checks.getByRole('link', { name: /checks.*failed/ }).click()
    await page.getByRole('heading', { name: 'Run run_checks' }).waitFor()
    assert.equal(new URL(page.url()).pathname, '/octo/demo/runs/run_checks')
    assert.deepEqual(errors, [])
  } finally {
    await browser.close()
    await server.close()
  }
})
