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

test('request CI separates workflows from jobs, folds completed results, and links Scope runs', async (t) => {
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
    const checks = page.getByRole('region', { name: 'CI' })

    await checks.getByText('CI running', { exact: true }).waitFor()
    await checks.getByText('· 3 of 8 left · 5 passed · 4 skipped', { exact: true }).waitFor()
    await checks.getByText('CI runs publicly, so this Agent request’s changes are public.', { exact: true }).waitFor()
    assert.equal(await checks.getByText(/GitHub|scope\/requests/).count(), 0)
    const workflow = checks.getByRole('region', { name: 'CI workflow', exact: true })
    await workflow.getByRole('heading', { name: 'CI Workflow', exact: true }).waitFor()
    assert.deepEqual(
      await workflow.getByRole('list', { name: 'CI jobs', exact: true }).getByRole('listitem').allTextContents(),
      ['Check operationsin progress', 'CLI validationValidate selected componentsqueued'],
    )
    await checks.getByRole('list', { name: 'Results without a workflow' }).getByText('Integration validation', { exact: true }).waitFor()
    assert.deepEqual(
      await workflow.getByRole('link').evaluateAll((links) => links.map((link) => link.getAttribute('href'))),
      ['/octo/demo/runs/9001', '/octo/demo/runs/9001#run-job-31', '/octo/demo/runs/9001#run-job-35'],
    )
    if (process.env.SCOPE_COMPONENT_SCREENSHOT) {
      await page.screenshot({ path: `${process.env.SCOPE_COMPONENT_SCREENSHOT}-ci-workflows.png`, fullPage: true })
    }

    await checks.getByRole('button', { name: 'Show all 12 results' }).click()
    await checks.getByRole('heading', { name: 'PR requirements Workflow', exact: true }).waitFor()
    assert.equal(await checks.getByRole('listitem').count(), 12)
    assert.equal(await workflow.getByRole('listitem').count(), 10)
    await checks.getByRole('region', { name: 'PR requirements workflow', exact: true }).getByRole('list', { name: 'PR requirements jobs' }).getByText('Required PR checks', { exact: true }).waitFor()
    if (process.env.SCOPE_COMPONENT_SCREENSHOT) {
      await page.screenshot({ path: `${process.env.SCOPE_COMPONENT_SCREENSHOT}-ci-workflows-expanded.png`, fullPage: true })
    }
    await page.evaluate(() => window.refresh())
    await checks.getByText('· 2 of 8 left · 6 passed · 4 skipped', { exact: true }).waitFor()
    assert.equal(await checks.getByRole('button', { name: 'Show less' }).getAttribute('aria-expanded'), 'true')

    await checks.getByRole('button', { name: 'Show less' }).click()
    await page.setViewportSize({ width: 390, height: 844 })
    assert.equal(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth), true)
    const job = checks.getByText('Integration validation', { exact: true })
    assert.equal(await job.evaluate((element) => element.scrollWidth <= element.clientWidth), true)
    assert.equal(await checks.getByText(/^Validate selected components/).first().isVisible(), true)
    if (process.env.SCOPE_COMPONENT_SCREENSHOT) {
      await page.screenshot({ path: `${process.env.SCOPE_COMPONENT_SCREENSHOT}-ci-workflows-mobile.png`, fullPage: true })
    }
    await page.setViewportSize({ width: 320, height: 740 })
    assert.equal(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth), true)
    await checks.getByRole('button', { name: 'Show all 12 results' }).click()
    await checks.getByRole('link', { name: /CLI validation/ }).click()
    await page.getByRole('heading', { name: 'Run 9001#run-job-35' }).waitFor()
    await page.goBack()

    await page.evaluate(() => window.showNative())
    await checks.getByText('CI failed', { exact: true }).waitFor()
    assert.equal(await checks.locator('a[href^="http"]').count(), 0)
    await checks.getByRole('link', { name: /checks.*failed/ }).click()
    await page.getByRole('heading', { name: 'Run run_checks' }).waitFor()
    assert.equal(new URL(page.url()).pathname, '/octo/demo/runs/run_checks')
    await page.goBack()
    await page.evaluate(() => window.showState('no-checks'))
    await checks.waitFor({ state: 'hidden' })
    assert.equal(await page.getByText(/CI passed|No CI required/).count(), 0)
    await page.evaluate(() => window.showState(null))
    await checks.getByText('CI status is not available yet.', { exact: true }).waitFor()
    await page.evaluate(() => window.showState('configuration-error'))
    await checks.getByText('The CI configuration for this revision is invalid.', { exact: true }).waitFor()
    assert.equal(await checks.getByText('CI passed', { exact: true }).count(), 0)
    await page.evaluate(() => window.showApproval())
    await page.getByRole('button', { name: 'Allow CI to run', exact: true }).click()
    const dialog = page.getByRole('alertdialog', { name: 'Allow CI to run?' })
    await dialog.getByText('a'.repeat(40), { exact: true }).waitFor()
    await dialog.getByText(/repository’s secrets/).waitFor()
    await dialog.getByText(/CI runs publicly/).waitFor()
    await page.evaluate(() => window.pushRevision())
    assert.equal(await dialog.getByText('a'.repeat(40), { exact: true }).isVisible(), true)
    assert.equal(await dialog.getByText('b'.repeat(40), { exact: true }).count(), 0)
    assert.equal(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth), true)
    if (process.env.SCOPE_COMPONENT_SCREENSHOT) {
      await page.screenshot({ path: `${process.env.SCOPE_COMPONENT_SCREENSHOT}-ci-permission.png`, fullPage: true })
    }
    await dialog.getByRole('button', { name: 'Allow CI to run', exact: true }).click()
    await dialog.waitFor({ state: 'hidden' })
    assert.equal(await page.evaluate(() => window.approvedHead), 'a'.repeat(40))
    assert.deepEqual(errors, [])
  } finally {
    await browser.close()
    await server.close()
  }
})
