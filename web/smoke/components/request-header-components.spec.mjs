import assert from 'node:assert/strict'
import test from 'node:test'
import { openFixture } from '../support/fixture-server.mjs'
import { visualBaseline } from '../support/visual-baseline.mjs'

const compareScreenshot = visualBaseline('request-header')

test('request header keeps metadata and actions aligned across container widths', async (t) => {
  const { base, errors, page } = await openFixture(t, 'request-header', { tailwind: true })
  await page.clock.setFixedTime(new Date('2026-11-06T12:00:00Z'))
  await page.goto(base, { waitUntil: 'domcontentloaded' })
  await page.evaluate(async () => {
    document.documentElement.classList.add('dark')
    await document.fonts.ready
  })
  const header = page.locator('.request-detail-header-secondary')
  const changes = page.getByRole('button', { name: 'Changes', exact: true })
  await changes.waitFor()
  for (const width of [1280, 1154, 1152, 390, 320]) {
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
  await page.setViewportSize({ width: 320, height: 700 })
  for (const name of [
    'Maintainers responsible for deployment approvals and repository access',
    'ReleaseEngineeringAndRepositorySecurityMaintainersWithoutAnyWordBreaks',
  ]) {
    await page.evaluate((viewName) => window.setViewName(viewName), name)
    await page.getByText(`${name} view`, { exact: true }).waitFor()
    assert.equal(await page.evaluate(() => document.documentElement.scrollWidth > innerWidth), false, 'Long view names stay inside the phone viewport')
  }
  await compareScreenshot(header, 'long-view')
  assert.deepEqual(errors, [])
})
