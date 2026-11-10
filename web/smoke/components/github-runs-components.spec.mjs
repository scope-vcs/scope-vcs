import assert from 'node:assert/strict'
import test from 'node:test'
import { fixtureClerk, fixtureRepositorySearch, openFixture } from '../support/fixture-server.mjs'
import { visualBaseline } from '../support/visual-baseline.mjs'

async function screenshot(page, options) {
  await page.mouse.move(0, 0)
  await page.evaluate(() => document.querySelector('main')?.scrollTo(0, 0))
  await page.evaluate(() => Promise.all(document.getAnimations()
    .filter((animation) => animation.effect?.getTiming().iterations !== Infinity)
    .map((animation) => animation.finished)))
  await page.screenshot(options)
}

const githubRunsFixture = { aliases: [fixtureClerk, fixtureRepositorySearch], tailwind: true }
const runPageRequestBudget = { listClick: 1, pageLoad: 1 }
const compareScreenshot = visualBaseline('run-page')
const requestsSoFar = (page) => page.evaluate(() => window.loads.length + window.runLoads.length + window.logLoads.length)

test('GitHub workflow runs open on Scope, keep their list across navigation and refresh in place', async (t) => {
  const { base, errors, page } = await openFixture(t, 'github-runs', githubRunsFixture)

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
  assert.deepEqual(await page.evaluate(() => window.nameLoads), [])

  await page.evaluate(() => window.clearLoads())
  await page.evaluate(() => window.setNextRuns({
    actions_url: 'https://github.com/octo/demo/actions',
    workflow_runs: [{
      id: 9, workflow_name: 'lint', branch: 'main', head_oid: 'b'.repeat(40), event: 'push',
      status: 'queued', conclusion: null, html_url: 'https://github.com/octo/demo/actions/runs/9',
      run_started_at_unix: null, updated_at_unix: Math.floor(Date.now() / 1000), request_id: null,
    }],
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

test('a GitHub run groups its jobs, opens the failed step at its error, and searches, folds and links log lines', async (t) => {
  const { base, errors, page } = await openFixture(t, 'github-runs', githubRunsFixture)
  const shots = process.env.SCOPE_COMPONENT_SCREENSHOT
  const jobs = page.getByRole('navigation', { name: 'Jobs' })
  const job = (name) => jobs.locator('button[aria-pressed]').filter({ hasText: new RegExp(`^${name.replace(/[()]/g, '\\$&')}`) })
  const step = (name) => page.getByRole('list', { name: 'Steps' }).getByRole('button', { name, exact: false })
  const output = (name) => page.getByRole('region', { name: `${name} output` })
  const testStep = 'Run the unit and integration test suites'
  const search = page.getByRole('searchbox', { name: 'Search log' })

  await page.goto(new URL('/octo/demo/runs', base).href, { timeout: 30_000 })
  await page.locator('main li').first().getByRole('link', { name: 'ci', exact: true }).click()
  await page.waitForURL('**/octo/demo/runs/1')
  await page.locator('h1', { hasText: 'ci' }).waitFor()
  await page.getByText('Loading jobs…').waitFor()
  assert.deepEqual(await page.evaluate(() => window.runLoads), ['1'])
  await page.evaluate(() => window.finishRunLoad())
  const failed = job('test (ubuntu-latest, node 24)')
  assert.equal(await failed.getAttribute('aria-pressed'), 'true')
  await page.getByText(`at ${testStep}`).waitFor()
  const error = output(testStep).locator('[data-line]').filter({ hasText: 'Process completed with exit code 1.' })
  await error.waitFor()
  assert.equal(await requestsSoFar(page), runPageRequestBudget.listClick,
    'a run opened from the list stays within its request budget before log text shows')

  assert.deepEqual(await page.getByRole('list', { name: 'Jobs by status' }).getByRole('listitem').allTextContents(),
    ['1 failed', '1 running', '7 passed'])
  const validate = jobs.getByRole('button', { name: /^Validate selected components/ })
  assert.equal(await validate.getAttribute('aria-expanded'), 'true')
  assert.match(await validate.textContent(), /1 failed · 7$/)
  const client = jobs.getByRole('button', { name: /^Client validation/ })
  assert.equal(await client.getAttribute('aria-expanded'), 'false')
  assert.equal(await job('Check desktop').count(), 0)
  await client.click()
  await job('Check desktop').waitFor()
  const filter = jobs.getByRole('searchbox', { name: 'Filter jobs' })
  await filter.fill('signed-in')
  const filtered = await jobs.locator('button[aria-pressed]').allTextContents()
  assert.equal(filtered.length, 1)
  assert.match(filtered[0], /^Signed-in contribution journey/)
  await filter.fill('')

  assert.equal(await step(testStep).getAttribute('aria-expanded'), 'true')
  assert.equal(await step('Set up job').getAttribute('aria-expanded'), 'false')
  await page.waitForFunction(() => {
    const line = [...document.querySelectorAll('[data-line]')].find((row) => row.textContent.includes('exit code 1'))
    const box = line?.getBoundingClientRect()
    return box && box.top >= 0 && box.bottom <= window.innerHeight
  })
  assert.equal(await error.evaluate((row) => getComputedStyle(row).backgroundColor !== 'rgba(0, 0, 0, 0)'), true)
  assert.equal(await output(testStep).getByText('##[').count(), 0)
  assert.equal(await output(testStep).locator('.text-success-strong', { hasText: '✔' }).count(), 40)
  const pnpmGroup = output(testStep).locator('[data-line]').filter({ hasText: 'Run pnpm test' })
  assert.match(await pnpmGroup.textContent(), /2 lines$/)
  assert.equal(await output(testStep).getByText('shell: /usr/bin/bash -e {0}').count(), 0)
  await pnpmGroup.getByRole('button', { name: 'Expand Run pnpm test' }).click()
  await output(testStep).getByText('shell: /usr/bin/bash -e {0}').waitFor()
  await pnpmGroup.getByRole('button', { name: 'Collapse Run pnpm test' }).click()
  assert.equal(await output(testStep).getByText(/\d{4}-\d{2}-\d{2}T\d{2}/).count(), 0)
  await page.getByRole('button', { name: 'Times', exact: true }).click()
  assert.match(await error.textContent(), /\d{2}:\d{2}:\d{2}Process completed/)
  await page.getByRole('button', { name: 'Times', exact: true }).click()
  assert.equal(
    await page.getByRole('link', { name: 'Full log' }).getAttribute('href'),
    'https://github.com/octo/demo/actions/runs/1/job/102',
  )
  assert.equal(
    await page.getByRole('link', { name: 'scope/requests/req_1', exact: true }).getAttribute('href'),
    '/octo/demo/requests/req_1',
  )
  assert.equal(await page.getByRole('link', { name: 'Open on GitHub' }).getAttribute('href'), 'https://github.com/octo/demo/actions/runs/1')

  await search.fill('Bash -E')
  await page.getByText('1 of 1', { exact: true }).waitFor()
  await output(testStep).locator('mark[data-current-match]', { hasText: 'bash -e' }).waitFor()
  await search.fill('octo/demo')
  await page.getByText('1 of 2', { exact: true }).waitFor()
  await output('Run actions/checkout@v4').locator('mark[data-current-match]', { hasText: 'octo/demo' }).waitFor()
  await search.press('Enter')
  await page.getByText('2 of 2', { exact: true }).waitFor()
  await output('Run actions/checkout@v4').getByText('Syncing repository: ').waitFor()
  await search.press('Shift+Enter')
  await page.getByText('1 of 2', { exact: true }).waitFor()
  await search.fill('')
  assert.equal(await output('Run actions/checkout@v4').count(), 0)

  const lineNumber = error.getByRole('link')
  const lineHash = `#run-job-102:3:L${await lineNumber.textContent()}`
  await lineNumber.click()
  assert.equal(await page.evaluate(() => location.hash), lineHash)

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
  await output('Build the web bundle and the API image').getByText('The log appears when this job finishes.').waitFor()
  assert.equal(await page.evaluate(() => location.hash), '#run-job-103')
  if (shots) {
    await page.setViewportSize({ width: 390, height: 844 })
    await screenshot(page, { fullPage: true, path: `${shots}.run-running-job-phone.png` })
    await page.setViewportSize({ width: 1280, height: 900 })
  }
  await failed.click()
  await error.waitFor()
  assert.deepEqual(await page.evaluate(() => window.logLoads), [])

  await job('build').click()
  await page.evaluate(() => {
    window.finishBuild()
    window.emitRunChanged(1)
  })
  await page.waitForFunction(() => window.runLoads.length === 2)
  assert.equal(await job('lint').count(), 1)
  await page.getByText('The log appears when this job finishes.').waitFor()
  await page.evaluate(() => window.finishRunLoad())
  await output('Build the web bundle and the API image').getByText('Build finished.').waitFor()
  assert.deepEqual(await page.evaluate(() => window.logLoads), ['103'])

  await page.goto(new URL('/octo/demo/runs', base).href)
  await page.goto(new URL('/octo/demo/runs/1#run-job-101', base).href)
  await output('Lint').getByText('Found 0 warnings and 0 errors.').waitFor()
  assert.equal(await requestsSoFar(page), runPageRequestBudget.pageLoad,
    'a run loaded directly stays within its request budget before log text shows')
  assert.equal(await job('lint').getAttribute('aria-pressed'), 'true')

  await page.goto(new URL('/octo/demo/runs', base).href)
  await page.goto(new URL(`/octo/demo/runs/1${lineHash}`, base).href)
  const linked = output(testStep).locator(`[data-line="${lineHash.split('L')[1]}"]`)
  await linked.waitFor()
  assert.equal(await failed.getAttribute('aria-pressed'), 'true')
  assert.match(await linked.getAttribute('class'), /outline-border-strong/)

  await page.setViewportSize({ width: 390, height: 844 })
  const picker = jobs.locator('button[aria-expanded][aria-controls]:not([aria-pressed])').first()
  await page.evaluate(() => { location.hash = '#run-job-104' })
  await page.getByText('Steps appear once a runner picks up this job.').waitFor()
  assert.equal(await job('deploy preview environment to the staging cluster').getAttribute('aria-pressed'), 'true')
  assert.match(await picker.textContent(), /deploy preview environment to the staging cluster/)
  assert.equal(await job('lint').isVisible(), false)
  const runLoadsBeforeDeploy = await page.evaluate(() => window.runLoads.length)
  await page.evaluate(() => {
    window.startDeploy()
    window.emitRunChanged(1)
  })
  await page.waitForFunction((loads) => window.runLoads.length > loads, runLoadsBeforeDeploy)
  await page.evaluate(() => window.finishRunLoad())
  await output('Deploy the preview').getByText('The log appears when this job finishes.').waitFor()
  assert.equal(await step('Set up job').getAttribute('aria-expanded'), 'false')
  await picker.click()
  await job('lint').click()
  await output('Lint').getByText('Found 0 warnings and 0 errors.').waitFor()
  assert.equal(await job('lint').isVisible(), false)
  if (shots) await screenshot(page, { fullPage: true, path: `${shots}.run-linked-queued-phone.png` })
  await page.setViewportSize({ width: 1280, height: 900 })

  await job('revoke preview token').click()
  await page.getByText('No steps ran.').waitFor()
  const log = page.getByRole('region', { name: 'Log' })
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

test('the run page keeps its reviewed look at desktop and phone widths in light and dark', async (t) => {
  const { base, errors, page } = await openFixture(t, 'github-runs', githubRunsFixture)
  await page.clock.setFixedTime(new Date('2026-11-06T12:00:00Z'))
  await page.goto(new URL('/octo/demo/runs/1', base).href, { timeout: 30_000 })
  await page.getByText('Process completed with exit code 1.').waitFor()
  await page.evaluate(() => document.fonts.ready)
  const pane = page.locator('.scope-content-enter')
  for (const theme of ['light', 'dark']) {
    await page.evaluate((dark) => document.documentElement.classList.toggle('dark', dark), theme === 'dark')
    for (const [width, height, name] of [[1280, 900, 'desktop'], [390, 3400, 'phone']]) {
      await page.setViewportSize({ width, height })
      await page.evaluate(() => window.scrollTo(0, 0))
      await compareScreenshot(pane, `${name}-${theme}`)
    }
  }
  assert.deepEqual(errors, [])
})

test('hovering a run starts its read, and its long included log lays out only the blocks on screen', async (t) => {
  const { base, errors, page } = await openFixture(t, 'github-runs', githubRunsFixture)
  const blocksOnScreen = () => page.evaluate(() => [...document.querySelectorAll('[data-log-block]')]
    .map((block, index) => block.firstElementChild.checkVisibility({ contentVisibilityAuto: true }) ? index : -1)
    .filter((index) => index >= 0))

  await page.goto(new URL('/octo/demo/runs', base).href, { timeout: 30_000 })
  const run = page.locator('main li').nth(2).getByRole('link', { name: 'ci', exact: true })
  await run.hover()
  await page.waitForFunction(() => window.runLoads.length === 1)
  assert.deepEqual(await page.evaluate(() => window.runLoads), ['3'])
  await page.evaluate(() => window.finishRunLoad())
  await run.click()
  await page.waitForURL('**/octo/demo/runs/3')
  const log = page.getByRole('region', { name: 'Package every artifact output' })
  await log.getByText('packaged artifact 0 into', { exact: false }).waitFor()
  assert.deepEqual(await page.evaluate(() => ({ runs: window.runLoads, logs: window.logLoads })), { runs: ['3'], logs: [] })

  assert.equal(await page.locator('[data-log-block]').count(), 105)
  await page.waitForFunction(() => document.querySelector('[data-log-block]').firstElementChild
    .checkVisibility({ contentVisibilityAuto: true }))
  const opened = await blocksOnScreen()
  assert.ok(opened.length > 0 && opened.length <= 3, `blocks laid out on open: ${opened}`)
  assert.equal(opened[0], 0)

  await page.evaluate(() => [...document.querySelectorAll('[data-log-block]')].at(-1).scrollIntoView({ block: 'end' }))
  await page.waitForFunction(() => [...document.querySelectorAll('[data-log-block]')].at(-1).firstElementChild
    .checkVisibility({ contentVisibilityAuto: true }))
  assert.ok((await blocksOnScreen()).includes(104))
  await log.getByText('packaged artifact 20999 into', { exact: false }).waitFor()
  assert.deepEqual(errors, [])
})
