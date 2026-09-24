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

test('auto-merge confirms one revision and stays usable on desktop and mobile', async (t) => {
  const cacheDir = await mkdtemp(join(tmpdir(), 'scope-vite-auto-merge-'))
  t.after(() => rm(cacheDir, { recursive: true, force: true }))
  const server = await createServer({
    cacheDir,
    configFile: false,
    root: fileURLToPath(new URL('./fixtures/request-auto-merge', import.meta.url)),
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
    await page.goto(server.resolvedUrls.local[0], {
      timeout: 30_000,
      waitUntil: 'domcontentloaded',
    })

    await page.getByRole('button', { name: 'Merge when checks pass' }).click()
    const dialog = page.getByRole('alertdialog')
    await dialog.getByText('a'.repeat(40), { exact: true }).waitFor()
    assert.equal(await dialog.getByText('revision-7d9f2f1b', { exact: true }).count(), 0)
    await page.evaluate(() => window.refreshAutoMerge())
    await dialog.getByText('a'.repeat(40), { exact: true }).waitFor()
    await dialog.getByRole('button', { name: 'Enable auto-merge' }).click()
    await page.getByText('Will merge when checks pass', { exact: true }).waitFor()
    assert.equal(
      await page.getByText(/Authorized by you/).textContent(),
      'Authorized by you · aaaaaaaaaaaa · Checks are still running.',
    )
    assert.deepEqual(await page.evaluate(() => window.calls[0]), {
      expected_head_oid: 'a'.repeat(40),
      expected_revision_id: 'revision-7d9f2f1b',
    })

    await page.setViewportSize({ width: 390, height: 844 })
    assert.equal(await page.evaluate(() => document.documentElement.scrollWidth > innerWidth), false)
    const footerBounds = await page.getByText('Will merge when checks pass', { exact: true })
      .locator('xpath=ancestor::div[1]').boundingBox()
    assert(footerBounds.x >= 0 && footerBounds.x + footerBounds.width <= 390)
    if (process.env.SCOPE_COMPONENT_SCREENSHOT) {
      await page.screenshot({ path: process.env.SCOPE_COMPONENT_SCREENSHOT, fullPage: true })
    }

    await page.getByRole('button', { name: 'Cancel auto-merge' }).click()
    const cancelDialog = page.getByRole('alertdialog')
    const dialogBounds = await cancelDialog.boundingBox()
    assert(dialogBounds.x >= 0 && dialogBounds.x + dialogBounds.width <= 390)
    await page.evaluate(() => window.refreshAutoMerge())
    await cancelDialog.getByText('aaaaaaaaaaaa → main', { exact: true }).waitFor()
    await cancelDialog.getByRole('button', { name: 'Cancel auto-merge' }).click()
    await page.getByRole('button', { name: 'Merge when checks pass' }).waitFor()
    // An ended authorization leaves the header; its record lives in the activity history.
    assert.equal(await page.getByText(/Authorized by/).count(), 0)
    assert.deepEqual(await page.evaluate(() => window.calls[1]), {
      expected_intent_id: 'intent-1',
    })
    await page.evaluate(() => window.setAutoMergeIntentStatus('Stopped'))
    await page.getByRole('button', { name: 'Merge when checks pass' }).waitFor()
    assert.equal(await page.getByText(/Authorized by/).count(), 0)
    await page.evaluate(() => window.showAutoMergeLoading())
    await page.locator('main > div.fixed').waitFor({ state: 'detached' })
    if (process.env.SCOPE_COMPONENT_SCREENSHOT) {
      await page.screenshot({
        path: `${process.env.SCOPE_COMPONENT_SCREENSHOT}.loading.png`,
        fullPage: true,
      })
    }
    assert.deepEqual(errors, [])
  } finally {
    await browser.close()
    await server.close()
  }
})
