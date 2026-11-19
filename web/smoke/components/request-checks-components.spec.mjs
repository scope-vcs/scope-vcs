import assert from 'node:assert/strict'
import test from 'node:test'
import { openFixture } from '../support/fixture-server.mjs'

const shot = async (target, name, options) => {
  if (process.env.SCOPE_COMPONENT_SCREENSHOT) {
    await target.screenshot({ animations: 'disabled', path: `${process.env.SCOPE_COMPONENT_SCREENSHOT}-${name}.png`, ...options })
  }
}

test('request CI shows one bar per job and one mark per review bot, and lists them on demand', async (t) => {
  const { base, errors, page } = await openFixture(t, 'request-checks', { tailwind: true })
  await page.goto(new URL('octo/demo/requests/req_1', base).href, { timeout: 30_000, waitUntil: 'domcontentloaded' })
  const checks = page.getByRole('region', { name: 'CI' })
  const jobs = page.getByRole('dialog', { name: 'CI jobs' })
  const workflow = jobs.getByRole('region', { name: 'CI workflow', exact: true })
  const bars = checks.locator('.request-check-bar')

  const running = checks.getByRole('button', { name: 'CI · 3 of 8 left · 5 passed · 4 skipped', exact: true })
  await running.waitFor()
  assert.deepEqual(
    await bars.evaluateAll((list) => list.map((bar) => bar.dataset.tone)),
    [...Array(5).fill('success'), 'running', 'waiting', 'waiting', ...Array(4).fill('inert')],
  )
  assert.equal(await checks.getByRole('link').count(), 0)
  assert.equal(await checks.getByText(/GitHub|scope\/requests|public/).count(), 0)
  await bars.nth(5).hover()
  await page.getByRole('tooltip', { name: 'Check operations running' }).waitFor()
  await shot(page, 'ci-bars', { fullPage: true })

  await running.click()
  await workflow.getByRole('heading', { name: 'CI', exact: true }).waitFor()
  const listed = await workflow.getByRole('list', { name: 'CI jobs', exact: true }).getByRole('listitem').allTextContents()
  assert.deepEqual(listed.slice(0, 2), ['Check operations', 'CLI validationValidate selected components'])
  assert.equal(listed.length, 10)
  assert.equal(await jobs.getByRole('listitem').count(), 12)
  await jobs.getByRole('list', { name: 'Results without a workflow' }).getByText('Integration validation', { exact: true }).waitFor()
  await jobs.getByRole('region', { name: 'PR requirements workflow', exact: true }).getByRole('list', { name: 'PR requirements jobs' }).getByText('Required PR checks', { exact: true }).waitFor()
  assert.deepEqual(
    (await workflow.getByRole('link').evaluateAll((links) => links.map((link) => link.getAttribute('href')))).slice(0, 3),
    ['/octo/demo/runs/9001', '/octo/demo/runs/9001#run-job-31', '/octo/demo/runs/9001#run-job-35'],
  )
  await shot(page, 'ci-jobs', { fullPage: true })
  await page.evaluate(() => window.refresh())
  await checks.getByRole('button', { name: 'CI · 2 of 8 left · 6 passed · 4 skipped', exact: true }).waitFor()
  assert.equal(await jobs.isVisible(), true)

  for (const width of [390, 320]) {
    await page.setViewportSize({ width, height: 844 })
    assert.equal(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth), true)
    const box = await jobs.boundingBox()
    assert(box.x >= 0 && box.x + box.width <= width, 'CI jobs stay within the phone viewport')
  }
  const job = jobs.getByText('Integration validation', { exact: true })
  assert.equal(await job.evaluate((element) => element.scrollWidth <= element.clientWidth), true)
  await shot(page, 'ci-jobs-mobile', { fullPage: true })
  await jobs.getByRole('link', { name: /CLI validation/ }).click()
  await page.getByRole('heading', { name: 'Run 9001#run-job-35' }).waitFor()
  await page.goBack()

  await page.evaluate(() => window.showFailing())
  await checks.getByRole('button', { name: 'CI · 1 failed · 1 passed', exact: true }).click()
  assert.equal(await jobs.getByRole('heading', { level: 3 }).count(), 0)
  assert.equal(await checks.locator('a[href^="http"]').count(), 0)
  await shot(page, 'ci-failed', { fullPage: true })
  await jobs.getByRole('link', { name: 'failed checks', exact: true }).click()
  await page.getByRole('heading', { name: 'Run 9003#run-job-51' }).waitFor()
  assert.equal(new URL(page.url()).pathname, '/octo/demo/runs/9003')
  await page.goBack()
  await page.evaluate(() => window.showState('no-checks'))
  await checks.waitFor({ state: 'hidden' })
  assert.equal(await page.getByText(/CI passed|No CI required/).count(), 0)
  await page.evaluate(() => window.showPullRequest({ state: 'opening', number: null, url: null, error: 'GitHub answered 502' }))
  await checks.getByText('Opening a pull request on GitHub', { exact: true }).waitFor()
  await checks.getByText('The last attempt failed: GitHub answered 502', { exact: true }).waitFor()
  assert.equal(await checks.getByRole('link').count(), 0)
  await page.evaluate(() => window.showPullRequest({ state: 'open', number: 12, url: 'https://github.com/octo/demo/pull/12', error: null }))
  const pullRequest = checks.getByRole('link', { name: 'Pull request #12 on GitHub', exact: true })
  assert.equal(await pullRequest.getAttribute('href'), 'https://github.com/octo/demo/pull/12')
  assert.equal(await pullRequest.getAttribute('target'), '_blank')
  assert.equal(await checks.getByText('The last attempt failed', { exact: false }).count(), 0)
  await page.evaluate(() => window.showPullRequest({ state: 'failed', number: 12, url: 'https://github.com/octo/demo/pull/12', error: 'The Scope GitHub App may not open pull requests in this GitHub repository.' }))
  await checks.getByRole('link', { name: 'Pull request #12 on GitHub is out of step', exact: true }).waitFor()
  await checks.getByText('The Scope GitHub App may not open pull requests in this GitHub repository.', { exact: true }).waitFor()
  assert.equal(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth), true)
  await shot(checks, 'ci-pull-request')
  await page.evaluate(() => window.showPullRequest(null))
  await checks.waitFor({ state: 'hidden' })

  const reviewers = page.getByRole('dialog', { name: 'Review bots' })
  await page.evaluate(() => window.showReviewBots([
    { login: 'greptile-apps[bot]', required: true, state: 'working' },
    { login: 'chatgpt-codex-connector[bot]', required: false, state: 'commented' },
    { login: 'octo-reviewer[bot]', required: true, state: 'clear' },
    { login: 'lint-bot[bot]', required: false, state: 'not_started' },
    { login: 'quiet-bot[bot]', required: true, state: 'timed_out' },
  ]))
  const reviewing = checks.getByRole('button', { name: 'Review bots · Reviewing', exact: true })
  await reviewing.waitFor()
  assert.deepEqual(
    await checks.locator('.request-review-bot').evaluateAll((marks) => marks.map((mark) => mark.dataset.state)),
    ['working', 'commented', 'clear', 'not_started', 'timed_out'],
  )
  assert.equal(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth), true)
  await shot(checks, 'ci-review-bots-narrow')
  await page.setViewportSize({ width: 1280, height: 800 })
  await checks.locator('.request-review-bot').nth(1).hover()
  await page.getByRole('tooltip', { name: 'chatgpt-codex-connector Finished with comments' }).waitFor()
  await reviewing.click()
  assert.deepEqual(
    await reviewers.getByRole('list', { name: 'Review bots', exact: true }).getByRole('listitem').evaluateAll((items) => items.map((item) => item.innerText.replace(/\s+/g, ' '))),
    [
      'G greptile-apps required Reviewing',
      'C chatgpt-codex-connector Finished with comments',
      'O octo-reviewer required Finished, nothing found',
      'L lint-bot Not started',
      'Q quiet-bot required Didn’t start · advisory',
    ],
  )
  assert.equal(await reviewers.getByRole('link').count(), 0)
  await shot(page, 'ci-review-bots', { fullPage: true })
  await page.keyboard.press('Escape')
  await page.setViewportSize({ width: 320, height: 740 })

  await page.evaluate(() => window.showPassed())
  await checks.getByRole('button', { name: 'CI · Checks passed', exact: true }).waitFor()
  assert.equal(await checks.getByRole('link').count(), 0)
  await checks.getByRole('button', { name: 'Review bots · Review clear', exact: true }).click()
  const carrier = reviewers.getByRole('link', { name: 'Pull request #12 on GitHub', exact: true })
  assert.equal(await carrier.getAttribute('href'), 'https://github.com/octo/demo/pull/12')
  assert.equal(await carrier.getAttribute('target'), '_blank')
  assert.equal(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth), true)
  await shot(page, 'ci-passed-mobile', { fullPage: true })
  await checks.getByRole('button', { name: 'CI · Checks passed', exact: true }).click()
  await jobs.getByRole('link', { name: 'succeeded lint', exact: true }).waitFor()
  assert.equal(await reviewers.count(), 0)
  await page.keyboard.press('Escape')
  await page.evaluate(() => window.showMany())
  await checks.getByRole('button', { name: 'Review bots · Not started', exact: true }).waitFor()
  assert.deepEqual([await bars.count(), await checks.locator('.request-review-bot').count()], [50, 20])
  assert.equal(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth), true)
  await shot(checks, 'ci-many-narrow')
  await page.evaluate(() => window.showState('no-checks'))
  await page.evaluate(() => window.showReviewBots([]))
  await page.evaluate(() => window.showPullRequest(null))
  await checks.waitFor({ state: 'hidden' })
  await page.evaluate(() => window.showState(null))
  await checks.getByText('CI status is not available yet.', { exact: true }).waitFor()
  await page.evaluate(() => window.showState('configuration-error'))
  await checks.getByText('The CI configuration for this revision is invalid.', { exact: true }).waitFor()
  assert.equal(await checks.getByText('CI passed', { exact: true }).count(), 0)
  await page.evaluate(() => window.showApproval())
  await checks.getByText('A maintainer must allow CI to run for this revision.', { exact: true }).waitFor()
  await checks.getByText('This revision changes GitHub workflow files. Allowing CI runs them with your repository’s secrets.', { exact: true }).waitFor()
  await page.getByRole('button', { name: 'Allow CI to run', exact: true }).click()
  const dialog = page.getByRole('alertdialog', { name: 'Allow CI to run?' })
  await dialog.getByText('a'.repeat(40), { exact: true }).waitFor()
  await dialog.getByText(/repository’s secrets/).waitFor()
  await dialog.getByText('CI runs publicly, so these changes are public.', { exact: true }).waitFor()
  await page.evaluate(() => window.pushRevision())
  assert.equal(await dialog.getByText('a'.repeat(40), { exact: true }).isVisible(), true)
  assert.equal(await dialog.getByText('b'.repeat(40), { exact: true }).count(), 0)
  assert.equal(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth), true)
  await shot(page, 'ci-permission', { fullPage: true })
  await dialog.getByRole('button', { name: 'Allow CI to run', exact: true }).click()
  await dialog.waitFor({ state: 'hidden' })
  assert.equal(await page.evaluate(() => window.approvedHead), 'a'.repeat(40))
  assert.deepEqual(errors, [])
})
