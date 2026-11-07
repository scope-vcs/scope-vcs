import assert from 'node:assert/strict'
import test from 'node:test'
import { openFixture } from '../support/fixture-server.mjs'

test('request CI keeps attention visible, discloses completed checks and review identity, and links Scope runs', async (t) => {
  const { base, errors, page } = await openFixture(t, 'request-checks', { tailwind: true })
  await page.goto(new URL('octo/demo/requests/req_1', base).href, { timeout: 30_000, waitUntil: 'domcontentloaded' })
  const checks = page.getByRole('region', { name: 'CI' })

  await checks.getByRole('heading', { name: 'CI · 3 of 8 left', exact: true }).waitFor()
  await checks.getByText('· 5 passed · 4 skipped', { exact: true }).waitFor()
  await checks.getByLabel('Changes are public', { exact: true }).waitFor()
  assert.equal(await checks.getByText('CI runs publicly, so these changes are public.', { exact: true }).isVisible(), false)
  assert.equal(await checks.getByText(/Workflow|CI running/).count(), 0)
  assert.equal(await checks.getByText(/GitHub|scope\/requests/).count(), 0)
  const workflow = checks.getByRole('region', { name: 'CI workflow', exact: true })
  await workflow.getByRole('heading', { name: 'CI', exact: true }).waitFor()
  assert.deepEqual(
    await workflow.getByRole('list', { name: 'CI jobs', exact: true }).getByRole('listitem').allTextContents(),
    ['Check operations', 'CLI validationValidate selected components'],
  )
  await checks.getByRole('list', { name: 'Results without a workflow' }).getByText('Integration validation', { exact: true }).waitFor()
  assert.deepEqual(
    await workflow.getByRole('link').evaluateAll((links) => links.map((link) => link.getAttribute('href'))),
    ['/octo/demo/runs/9001', '/octo/demo/runs/9001#run-job-31', '/octo/demo/runs/9001#run-job-35'],
  )
  if (process.env.SCOPE_COMPONENT_SCREENSHOT) {
    await page.screenshot({ animations: 'disabled', path: `${process.env.SCOPE_COMPONENT_SCREENSHOT}-ci-workflows.png`, fullPage: true })
  }

  await checks.getByRole('button', { name: 'Details' }).click()
  await checks.getByText('CI runs publicly, so these changes are public.', { exact: true }).waitFor()
  await checks.getByRole('heading', { name: 'PR requirements', exact: true }).waitFor()
  assert.equal(await checks.getByRole('listitem').count(), 12)
  assert.equal(await workflow.getByRole('listitem').count(), 10)
  await checks.getByRole('region', { name: 'PR requirements workflow', exact: true }).getByRole('list', { name: 'PR requirements jobs' }).getByText('Required PR checks', { exact: true }).waitFor()
  if (process.env.SCOPE_COMPONENT_SCREENSHOT) {
    await page.screenshot({ animations: 'disabled', path: `${process.env.SCOPE_COMPONENT_SCREENSHOT}-ci-workflows-expanded.png`, fullPage: true })
  }
  await page.evaluate(() => window.refresh())
  await checks.getByText('· 6 passed · 4 skipped', { exact: true }).waitFor()
  await checks.getByRole('heading', { name: 'CI · 2 of 8 left', exact: true }).waitFor()
  assert.equal(await checks.getByRole('button', { name: 'Details' }).getAttribute('aria-expanded'), 'true')

  await checks.getByRole('button', { name: 'Details' }).click()
  await page.setViewportSize({ width: 390, height: 844 })
  assert.equal(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth), true)
  const job = checks.getByText('Integration validation', { exact: true })
  assert.equal(await job.evaluate((element) => element.scrollWidth <= element.clientWidth), true)
  assert.equal(await checks.getByText(/^Validate selected components/).first().isVisible(), true)
  if (process.env.SCOPE_COMPONENT_SCREENSHOT) {
    await page.screenshot({ animations: 'disabled', path: `${process.env.SCOPE_COMPONENT_SCREENSHOT}-ci-workflows-mobile.png`, fullPage: true })
  }
  await page.setViewportSize({ width: 320, height: 740 })
  assert.equal(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth), true)
  await checks.getByRole('button', { name: 'Details' }).click()
  await checks.getByRole('link', { name: /CLI validation/ }).click()
  await page.getByRole('heading', { name: 'Run 9001#run-job-35' }).waitFor()
  await page.goBack()

  await page.evaluate(() => window.showNative())
  await checks.getByRole('heading', { name: 'CI · 1 failed', exact: true }).waitFor()
  assert.equal(await checks.getByRole('heading', { level: 3 }).count(), 0)
  assert.equal(await checks.locator('a[href^="http"]').count(), 0)
  if (process.env.SCOPE_COMPONENT_SCREENSHOT) {
    await page.screenshot({ animations: 'disabled', path: `${process.env.SCOPE_COMPONENT_SCREENSHOT}-ci-failed.png`, fullPage: true })
  }
  await checks.getByRole('link', { name: 'failed checks', exact: true }).click()
  await page.getByRole('heading', { name: 'Run run_checks' }).waitFor()
  assert.equal(new URL(page.url()).pathname, '/octo/demo/runs/run_checks')
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
  if (process.env.SCOPE_COMPONENT_SCREENSHOT) {
    await checks.screenshot({ animations: 'disabled', path: `${process.env.SCOPE_COMPONENT_SCREENSHOT}-ci-pull-request.png` })
  }
  await page.evaluate(() => window.showPullRequest(null))
  await checks.waitFor({ state: 'hidden' })
  await page.evaluate(() => window.showReviewBots([
    { login: 'greptile-apps[bot]', required: true, state: 'working' },
    { login: 'chatgpt-codex-connector[bot]', required: false, state: 'commented' },
    { login: 'octo-reviewer[bot]', required: true, state: 'clear' },
    { login: 'lint-bot[bot]', required: false, state: 'not_started' },
    { login: 'quiet-bot[bot]', required: true, state: 'timed_out' },
  ]))
  await checks.getByText('Reviewing · Finished with comments · Not started · Didn’t start · advisory', { exact: true }).waitFor()
  assert.equal(await checks.getByText('Review clear', { exact: true }).count(), 0)
  assert.equal(await checks.getByRole('list', { name: 'Review bots', exact: true }).isVisible(), false)
  await checks.getByRole('button', { name: 'Details' }).click()
  assert.deepEqual(
    await checks.getByRole('list', { name: 'Review bots', exact: true }).getByRole('listitem').allTextContents(),
    [
      'greptile-apps[bot]requiredReviewing',
      'chatgpt-codex-connector[bot]Finished with comments',
      'octo-reviewer[bot]requiredFinished, nothing found',
      'lint-bot[bot]Not started',
      'quiet-bot[bot]requiredDidn’t start · advisory',
    ],
  )
  assert.equal(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth), true)
  if (process.env.SCOPE_COMPONENT_SCREENSHOT) {
    await checks.screenshot({ animations: 'disabled', path: `${process.env.SCOPE_COMPONENT_SCREENSHOT}-ci-review-bots-narrow.png` })
    await page.setViewportSize({ width: 1280, height: 800 })
    await checks.screenshot({ animations: 'disabled', path: `${process.env.SCOPE_COMPONENT_SCREENSHOT}-ci-review-bots.png` })
    await page.setViewportSize({ width: 320, height: 740 })
  }
  await checks.getByRole('button', { name: 'Details' }).click()
  await page.evaluate(() => window.showPassed())
  await checks.getByRole('heading', { name: 'Checks passed', exact: true }).waitFor()
  await checks.getByText('· Review clear', { exact: true }).waitFor()
  assert.equal(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth), true)
  assert.equal(await checks.getByRole('link', { name: 'succeeded lint', exact: true }).count(), 0)
  if (process.env.SCOPE_COMPONENT_SCREENSHOT) {
    await checks.screenshot({ animations: 'disabled', path: `${process.env.SCOPE_COMPONENT_SCREENSHOT}-ci-passed-mobile.png` })
    await page.setViewportSize({ width: 1280, height: 800 })
    await checks.screenshot({ animations: 'disabled', path: `${process.env.SCOPE_COMPONENT_SCREENSHOT}-ci-passed.png` })
    await page.setViewportSize({ width: 320, height: 740 })
  }
  await checks.getByRole('button', { name: 'Details' }).click()
  await checks.getByRole('link', { name: 'succeeded lint', exact: true }).waitFor()
  await checks.getByText('octo-reviewer[bot]', { exact: true }).waitFor()
  await checks.getByRole('button', { name: 'Details' }).click()
  await page.evaluate(() => window.showState('no-checks'))
  await page.evaluate(() => window.showReviewBots([]))
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
  if (process.env.SCOPE_COMPONENT_SCREENSHOT) {
    await page.screenshot({ animations: 'disabled', path: `${process.env.SCOPE_COMPONENT_SCREENSHOT}-ci-permission.png`, fullPage: true })
  }
  await dialog.getByRole('button', { name: 'Allow CI to run', exact: true }).click()
  await dialog.waitFor({ state: 'hidden' })
  assert.equal(await page.evaluate(() => window.approvedHead), 'a'.repeat(40))
  assert.deepEqual(errors, [])
})
