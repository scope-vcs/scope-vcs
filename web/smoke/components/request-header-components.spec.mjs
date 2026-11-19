import assert from 'node:assert/strict'
import test from 'node:test'
import { openFixture } from '../support/fixture-server.mjs'
import { visualBaseline } from '../support/visual-baseline.mjs'

const compareScreenshot = visualBaseline('request-header')

test('request header keeps its state, facts, actions, and checks aligned across container widths', async (t) => {
  const { base, browser, errors, page } = await openFixture(t, 'request-header', { tailwind: true })
  await page.clock.setFixedTime(new Date('2026-11-06T12:00:00Z'))
  await page.goto(base, { waitUntil: 'domcontentloaded' })
  await page.evaluate(async () => {
    document.documentElement.classList.add('dark')
    await document.fonts.ready
  })
  const header = page.locator('.request-detail-pane')
  const changes = page.getByRole('button', { name: /^Changes/ })
  await changes.waitFor()
  for (const width of [1280, 490, 486, 390, 320]) {
    await page.setViewportSize({ width, height: 700 })
    await page.evaluate(() => window.setBranch('repository-row-security'))
    await page.getByText('repository-row-security', { exact: true }).waitFor()
    await compareScreenshot(header, `header-${width}`)
    assert.equal(await page.evaluate(() => document.documentElement.scrollWidth > innerWidth), false)
  }
  await page.setViewportSize({ width: 1280, height: 700 })
  await page.evaluate(() => window.setBranch('repository-row-security-with-a-long-name-that-must-not-displace-actions'))
  await page.getByText('repository-row-security-with-a-long-name-that-must-not-displace-actions', { exact: true }).waitFor()
  await compareScreenshot(header, 'long-branch')
  for (const state of ['failed', 'merged']) {
    await page.evaluate((name) => window.setState(name), state)
    await page.getByText(state === 'failed' ? 'CI failed' : 'Merged', { exact: true }).waitFor()
    await compareScreenshot(header, state)
  }
  assert.equal(
    await page.getByRole('link', { name: 'Required PR checks', exact: true }).count(),
    0,
  )
  await page.evaluate(() => window.setState('failed'))
  assert.equal(
    await page.getByRole('link', { name: 'Required PR checks', exact: true }).getAttribute('href'),
    '/adamblumoff/scope-vcs/runs/9002#run-job-45',
  )
  await page.setViewportSize({ width: 320, height: 700 })
  await page.evaluate(() => window.setState('longFailure'))
  await page.getByRole('link', { name: /^required_web_validation_/ }).waitFor()
  assert.equal(await page.evaluate(() => document.documentElement.scrollWidth > innerWidth), false)
  await page.setViewportSize({ width: 1280, height: 700 })
  await page.evaluate(() => window.setState('findings'))
  await compareScreenshot(changes, 'changes-chevron')
  await changes.click()
  const menu = page.getByRole('dialog', { name: 'Request changes' })
  await menu.getByText('No pushes in recent activity.').waitFor()
  await compareScreenshot(menu, 'changes-menu')
  const triggerBox = await changes.boundingBox()
  const menuBox = await menu.boundingBox()
  assert(Math.abs(menuBox.y - triggerBox.y - triggerBox.height - 4) <= 1, 'Changes menu opens directly below its trigger')
  await page.keyboard.press('Escape')
  assert.equal(await menu.count(), 0)
  await page.setViewportSize({ width: 390, height: 700 })
  await changes.click()
  await menu.getByText('No pushes in recent activity.').waitFor()
  const box = await menu.boundingBox()
  assert(box.x >= 0 && box.x + box.width <= 390, 'Changes menu stays within the phone viewport')
  await page.keyboard.press('Escape')
  await page.setViewportSize({ width: 1280, height: 700 })
  await changes.hover()
  await page.getByRole('tooltip', { name: 'Changes · 203 files' }).waitFor()
  const tooltipName = 'CI runs publicly, so these changes are public.'
  await page.getByRole('button', { name: 'Public', exact: true }).hover()
  await page.getByRole('tooltip', { name: tooltipName }).waitFor()
  await page.getByRole('button', { name: 'More request actions', exact: true }).click()
  assert.equal(
    await page.getByRole('dialog', { name: 'More request actions' }).getByRole('link', { name: 'Browse code', exact: true }).getAttribute('href'),
    '/adamblumoff/scope-vcs/code?request=request',
  )

  const phone = await browser.newPage({ hasTouch: true, viewport: { width: 320, height: 700 } })
  await phone.goto(base, { waitUntil: 'domcontentloaded' })
  await phone.getByRole('button', { name: 'Public', exact: true }).tap()
  const publicTooltip = phone.getByRole('tooltip', { name: tooltipName })
  await publicTooltip.waitFor()
  await phone.getByRole('heading', { name: 'Enforce repository access', exact: true }).tap()
  await publicTooltip.waitFor({ state: 'hidden' })
  assert.deepEqual(errors, [])
})
