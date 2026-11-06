import assert from 'node:assert/strict'
import { mkdir, readFile, writeFile } from 'node:fs/promises'
import test from 'node:test'
import { PNG } from 'pngjs'
import pixelmatch from 'pixelmatch'
import { openFixture } from '../support/fixture-server.mjs'

const snapshots = new URL('../snapshots/request-header/', import.meta.url)
const artifacts = new URL('../../.tmp/request-header/', import.meta.url)

async function compareScreenshot(locator, name) {
  const actualBytes = await locator.screenshot({ animations: 'disabled' })
  if (process.env.SCOPE_UPDATE_HEADER_SNAPSHOTS === '1') {
    assert(!process.env.CI, 'CI cannot update visual baselines')
    await writeFile(new URL(`${name}.png`, snapshots), actualBytes)
    return
  }
  const actual = PNG.sync.read(actualBytes)
  const expected = PNG.sync.read(await readFile(new URL(`${name}.png`, snapshots)))
  await mkdir(artifacts, { recursive: true })
  await writeFile(new URL(`${name}-actual.png`, artifacts), actualBytes)
  assert.deepEqual([actual.width, actual.height], [expected.width, expected.height], `${name}: layout dimensions changed`)
  const diff = new PNG({ width: actual.width, height: actual.height })
  const changed = pixelmatch(expected.data, actual.data, diff.data, actual.width, actual.height, { threshold: 0.15 })
  if (changed) await writeFile(new URL(`${name}-diff.png`, artifacts), PNG.sync.write(diff))
  assert.equal(changed, 0, `${name}: rendered pixels changed; inspect web/.tmp/request-header before updating the baseline`)
}

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
  assert.deepEqual(errors, [])
})
