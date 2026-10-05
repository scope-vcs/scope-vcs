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

test('GitHub workflow runs link out, keep their list across navigation and refresh in place', async (t) => {
  const cacheDir = await mkdtemp(join(tmpdir(), 'scope-vite-github-runs-'))
  t.after(() => rm(cacheDir, { recursive: true, force: true }))
  const server = await createServer({
    cacheDir,
    configFile: false,
    root: fileURLToPath(new URL('./fixtures/github-runs', import.meta.url)),
    plugins: [tailwindcss()],
    server: { host: '127.0.0.1', port: 0, fs: { allow: [fileURLToPath(new URL('..', import.meta.url))] } },
    resolve: { alias: [
      { find: '@clerk/tanstack-react-start', replacement: fileURLToPath(new URL('./fixtures/request-workspace/clerk.tsx', import.meta.url)) },
      { find: '@', replacement: fileURLToPath(new URL('../src', import.meta.url)) },
      ...['react/jsx-dev-runtime', 'react/jsx-runtime', 'react-dom/client', 'react']
        .map((name) => ({ find: name, replacement: require.resolve(name) })),
    ] },
    oxc: { jsx: { runtime: 'automatic' } },
  })
  t.after(() => server.close())
  await server.listen()
  const browser = await chromium.launch({ headless: true })
  t.after(() => browser.close())
  const page = await browser.newPage({ viewport: { width: 1280, height: 900 } })
  page.setDefaultTimeout(10_000)
  const errors = []
  page.on('pageerror', (error) => errors.push(error.message))
  const base = server.resolvedUrls.local[0]

  await page.goto(new URL('/octo/demo/runs', base).href, { timeout: 30_000 })
  const rows = page.locator('main li')
  await rows.first().waitFor()
  assert.equal(await rows.count(), 3)
  const ci = rows.first().getByRole('link', { name: 'ci', exact: true })
  assert.equal(await ci.getAttribute('href'), 'https://github.com/octo/demo/actions/runs/1')
  assert.equal(await ci.getAttribute('target'), '_blank')
  assert.equal(
    await page.getByRole('link', { name: 'All runs on GitHub' }).getAttribute('href'),
    'https://github.com/octo/demo/actions',
  )
  for (const [width, name] of [[1280, 'desktop'], [390, 'phone']]) {
    await page.setViewportSize({ width, height: 844 })
    assert.equal(
      await page.evaluate(() => document.documentElement.scrollWidth <= window.innerWidth),
      true,
      `${name} has no horizontal scroll`,
    )
    if (process.env.SCOPE_COMPONENT_SCREENSHOT) {
      await page.screenshot({ path: `${process.env.SCOPE_COMPONENT_SCREENSHOT}.runs-${name}.png` })
    }
  }
  await page.setViewportSize({ width: 1280, height: 900 })

  await rows.first().getByRole('link', { name: 'scope/requests/req_1', exact: true }).click()
  await page.getByRole('heading', { name: 'Request req_1' }).waitFor()
  await page.getByRole('link', { name: 'Back to runs' }).click()
  await rows.first().waitFor()
  assert.equal(await rows.count(), 3)
  assert.deepEqual(await page.evaluate(() => window.loads), [])

  const older = page.getByRole('button', { name: 'Load older runs' })
  await older.click()
  await page.waitForFunction(() => window.loads.length === 1)
  assert.deepEqual(await page.evaluate(() => window.loads), ['all after page-2'])
  assert.equal(await rows.count(), 3)
  await page.evaluate(() => window.finishLoad())
  await page.getByText('Showing 5', { exact: true }).waitFor()
  assert.equal(await older.count(), 0)

  const filter = page.getByRole('combobox', { name: 'Filter by workflow' })
  await filter.selectOption('lint')
  await page.getByRole('list', { name: 'Loading runs' }).waitFor()
  await page.evaluate(() => window.finishLoad())
  await page.getByText('Showing 1', { exact: true }).waitFor()
  assert.equal(await rows.first().getByRole('link', { name: 'lint', exact: true }).count(), 1)
  for (const [width, name] of [[1280, 'desktop'], [390, 'phone']]) {
    await page.setViewportSize({ width, height: 844 })
    assert.equal(
      await page.evaluate(() => document.documentElement.scrollWidth <= window.innerWidth),
      true,
      `${name} filter has no horizontal scroll`,
    )
    if (process.env.SCOPE_COMPONENT_SCREENSHOT) {
      await page.screenshot({ path: `${process.env.SCOPE_COMPONENT_SCREENSHOT}.runs-filter-${name}.png` })
    }
  }
  await page.setViewportSize({ width: 1280, height: 900 })
  await filter.selectOption('')
  await page.getByText('Showing 5', { exact: true }).waitFor()
  assert.deepEqual(await page.evaluate(() => window.loads), ['all after page-2', 'lint'])

  await page.evaluate(() => window.clearLoads())
  await page.evaluate(() => window.setNextRuns({
    actions_url: 'https://github.com/octo/demo/actions',
    workflow_runs: [{
      id: 9, workflow_name: 'lint', branch: 'main', head_oid: 'b'.repeat(40), event: 'push',
      status: 'queued', conclusion: null, html_url: 'https://github.com/octo/demo/actions/runs/9',
      run_started_at_unix: null, updated_at_unix: Math.floor(Date.now() / 1000), request_id: null,
    }],
    workflows: ['lint'],
    next_cursor: null,
  }))
  await page.evaluate(() => window.emitRunsChanged())
  await page.waitForFunction(() => window.loads.length === 1)
  assert.equal(await rows.count(), 5)
  await page.evaluate(() => window.finishLoad())
  await page.getByRole('link', { name: 'lint', exact: true }).waitFor()
  assert.equal(await rows.count(), 1)

  const sentence = page.getByText('Runs come from this project’s GitHub Actions workflows once GitHub is connected.')
  const connect = page.getByRole('button', { name: 'Connect GitHub', exact: true })
  await page.goto(new URL('/octo/demo/runs-empty', base).href)
  await sentence.waitFor()
  await connect.waitFor()
  for (const [width, name] of [[1280, 'desktop'], [390, 'phone']]) {
    await page.setViewportSize({ width, height: 844 })
    assert.equal(
      await page.evaluate(() => document.documentElement.scrollWidth <= window.innerWidth),
      true,
      `${name} has no horizontal scroll`,
    )
    if (process.env.SCOPE_COMPONENT_SCREENSHOT) {
      await page.screenshot({ path: `${process.env.SCOPE_COMPONENT_SCREENSHOT}.connect-${name}.png` })
    }
  }
  await page.setViewportSize({ width: 1280, height: 900 })
  await connect.click()
  await page.waitForFunction(() => location.hash === '#github-authorize')
  assert.deepEqual(await page.evaluate(() => window.authorizeCalls), [{ owner: 'octo', repo: 'demo' }])
  assert.equal(
    await page.evaluate(() => sessionStorage.getItem('scope.github-setup.return-path')),
    '/octo/demo/runs-empty',
  )

  await page.evaluate(() => window.setActor('Public'))
  await sentence.waitFor()
  assert.equal(await connect.count(), 0)

  await page.goto(new URL('/octo/demo/runs-empty?configured=false', base).href)
  await page.getByText('Push to main with a matching trigger', { exact: false }).waitFor()
  assert.equal(await sentence.count(), 0)
  assert.equal(await connect.count(), 0)

  await page.goto(new URL('/octo/demo/runs-connected', base).href)
  const test = page.getByRole('link', { name: 'Test connection', exact: true })
  await test.waitFor()
  assert.equal(await test.getAttribute('href'), '/octo/demo/settings#ci')
  if (process.env.SCOPE_COMPONENT_SCREENSHOT) {
    await page.setViewportSize({ width: 390, height: 844 })
    await page.screenshot({ path: `${process.env.SCOPE_COMPONENT_SCREENSHOT}.connected-empty-phone.png` })
    await page.setViewportSize({ width: 1280, height: 900 })
  }
  await test.click()
  await page.getByRole('heading', { name: 'Settings' }).waitFor()
  assert.deepEqual(errors, [])
})
