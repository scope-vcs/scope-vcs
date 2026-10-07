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

async function screenshot(page, options) {
  await page.mouse.move(0, 0)
  await page.evaluate(() => Promise.all(document.getAnimations()
    .filter((animation) => animation.effect?.getTiming().iterations !== Infinity)
    .map((animation) => animation.finished)))
  await page.screenshot(options)
}

async function openFixture(t) {
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
  return { base: server.resolvedUrls.local[0], errors, page }
}

test('GitHub workflow runs open on Scope, keep their list across navigation and refresh in place', async (t) => {
  const { base, errors, page } = await openFixture(t)

  await page.goto(new URL('/octo/demo/runs', base).href, { timeout: 30_000 })
  const rows = page.locator('main li')
  await rows.first().waitFor()
  assert.equal(await rows.count(), 3)
  const ci = rows.first().getByRole('link', { name: 'ci', exact: true })
  assert.equal(await ci.getAttribute('href'), '/octo/demo/runs/1')
  assert.equal(await ci.getAttribute('target'), null)
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
      await screenshot(page, { path: `${process.env.SCOPE_COMPONENT_SCREENSHOT}.runs-${name}.png` })
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
      await screenshot(page, { path: `${process.env.SCOPE_COMPONENT_SCREENSHOT}.runs-filter-${name}.png` })
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
      await screenshot(page, { path: `${process.env.SCOPE_COMPONENT_SCREENSHOT}.connect-${name}.png` })
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
    await screenshot(page, { path: `${process.env.SCOPE_COMPONENT_SCREENSHOT}.connected-empty-phone.png` })
    await page.setViewportSize({ width: 1280, height: 900 })
  }
  await test.click()
  await page.getByRole('heading', { name: 'Settings' }).waitFor()
  assert.deepEqual(errors, [])
})

test('a GitHub run shows its jobs, steps and finished logs, follows job links and refreshes in place', async (t) => {
  const { base, errors, page } = await openFixture(t)
  const shots = process.env.SCOPE_COMPONENT_SCREENSHOT
  const job = (name) => page.getByRole('navigation', { name: 'Jobs' }).getByRole('button').filter({ hasText: name })
  const log = page.getByRole('region', { name: 'Log' })

  await page.goto(new URL('/octo/demo/runs', base).href, { timeout: 30_000 })
  await page.locator('main li').first().getByRole('link', { name: 'ci', exact: true }).click()
  await page.waitForURL('**/octo/demo/runs/1')
  await page.locator('h1', { hasText: 'ci' }).waitFor()
  await page.getByText('Loading jobs…').waitFor()
  assert.deepEqual(await page.evaluate(() => window.runLoads), ['1'])
  await page.evaluate(() => window.finishRunLoad())
  const failed = job('test (ubuntu-latest, node 24)')
  assert.equal(await failed.getAttribute('aria-pressed'), 'true')
  await page.getByText('at Run the unit and integration test suites').waitFor()
  await log.getByText('Process completed with exit code 1.', { exact: false }).waitFor()
  assert.equal((await log.locator('pre').textContent()).includes('2026-10-05T'), false)
  assert.equal(
    await log.getByRole('link', { name: 'Full log' }).getAttribute('href'),
    'https://github.com/octo/demo/actions/runs/1/job/102',
  )
  assert.equal(
    await page.getByRole('link', { name: 'scope/requests/req_1', exact: true }).getAttribute('href'),
    '/octo/demo/requests/req_1',
  )
  for (const theme of ['light', 'dark']) {
    await page.evaluate((dark) => document.documentElement.classList.toggle('dark', dark), theme === 'dark')
    for (const [width, height, name] of [[1280, 900, 'desktop'], [390, 844, 'phone']]) {
      await page.setViewportSize({ width, height })
      assert.equal(
        await page.evaluate(() => document.documentElement.scrollWidth <= window.innerWidth),
        true,
        `${name} run page has no horizontal scroll`,
      )
      if (shots) await screenshot(page, { fullPage: true, path: `${shots}.run-${theme}-${name}.png` })
    }
  }
  await page.evaluate(() => document.documentElement.classList.remove('dark'))
  await page.setViewportSize({ width: 1280, height: 900 })

  await job('build').click()
  await log.getByText('The log appears when this job finishes.').waitFor()
  assert.equal(await page.evaluate(() => location.hash), '#run-job-103')
  if (shots) {
    await page.setViewportSize({ width: 390, height: 844 })
    await screenshot(page, { fullPage: true, path: `${shots}.run-running-job-phone.png` })
    await page.setViewportSize({ width: 1280, height: 900 })
  }
  await failed.click()
  await log.getByText('Process completed with exit code 1.', { exact: false }).waitFor()
  assert.deepEqual(await page.evaluate(() => window.logLoads), ['102'])

  await job('build').click()
  await page.evaluate(() => {
    window.finishBuild()
    window.emitRunChanged(1)
  })
  await page.waitForFunction(() => window.runLoads.length === 2)
  assert.equal(await job('lint').count(), 1)
  await log.getByText('The log appears when this job finishes.').waitFor()
  await page.evaluate(() => window.finishRunLoad())
  await log.getByText('Build finished.').waitFor()
  assert.deepEqual(await page.evaluate(() => window.logLoads), ['102', '103'])

  await page.goto(new URL('/octo/demo/runs', base).href)
  await page.goto(new URL('/octo/demo/runs/1#run-job-101', base).href)
  await log.getByText('Found 0 warnings and 0 errors.').waitFor()
  assert.equal(await job('lint').getAttribute('aria-pressed'), 'true')
  await page.setViewportSize({ width: 390, height: 844 })
  await page.evaluate(() => { location.hash = '#run-job-104' })
  await page.getByText('Steps appear once a runner picks up this job.').waitFor()
  assert.equal(await job('deploy preview environment to the staging cluster').getAttribute('aria-pressed'), 'true')
  await page.waitForFunction(() => {
    const button = document.querySelector('button[aria-controls="run-job-104"]')
    const list = button?.closest('ul')
    if (!button || !list) return false
    const shown = list.getBoundingClientRect()
    const row = button.getBoundingClientRect()
    return row.left >= shown.left - 1 && row.right <= shown.right + 1
  })
  if (shots) await screenshot(page, { fullPage: true, path: `${shots}.run-linked-queued-phone.png` })
  await page.setViewportSize({ width: 1280, height: 900 })

  await job('revoke preview token').click()
  await page.getByText('No steps ran.').waitFor()
  await log.getByText('This job was skipped, so it has no log.').waitFor()
  assert.equal(await log.getByRole('button', { name: 'Retry' }).count(), 0)
  if (shots) await screenshot(page, { fullPage: true, path: `${shots}.run-skipped-job.png` })

  await page.goto(new URL('/octo/demo/runs/2', base).href)
  await page.getByText('Loading jobs…').waitFor()
  if (shots) {
    for (const [width, height, name] of [[1280, 900, 'desktop'], [390, 844, 'phone']]) {
      await page.setViewportSize({ width, height })
      await screenshot(page, { fullPage: true, path: `${shots}.run-jobs-loading-${name}.png` })
    }
  }
  assert.deepEqual(errors, [])
})
