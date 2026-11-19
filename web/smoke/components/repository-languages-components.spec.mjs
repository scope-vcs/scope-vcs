import assert from 'node:assert/strict'
import test from 'node:test'
import { openFixture } from '../support/fixture-server.mjs'

const summary = 'Languages: Rust 59.5%, TypeScript 28.4%, JavaScript 8.5%, HTML 1.0%, SQL 1.0%, Python 0.7%, Other 0.9%'

async function shownList(page) {
  const list = page.locator('[data-slot="tooltip-content"]')
  await list.waitFor()
  return {
    bold: await list.locator('li.font-semibold').allTextContents(),
    rows: await list.locator('li').allTextContents(),
  }
}

async function screenshot(page, name) {
  if (process.env.SCOPE_COMPONENT_SCREENSHOT) {
    await page.screenshot({ path: `${process.env.SCOPE_COMPONENT_SCREENSHOT}.${name}.png` })
  }
}

test('the language bar lists languages on hover and keyboard focus, and keeps every part visible', async (t) => {
  const { base, errors, page } = await openFixture(t, 'repository-languages', { tailwind: true })
  await page.goto(base)
  const first = page.getByRole('button', { name: summary, exact: true })
  await first.waitFor()
  const segments = page.locator('main > div > button')
  assert.equal(await segments.count(), 7)
  const widths = await segments.evaluateAll((nodes) => nodes.map((node) => node.getBoundingClientRect().width))
  assert.ok(widths.every((width) => width >= 3), `${widths}`)
  assert.ok(widths[0] > widths[1] && widths[1] > widths[2], `${widths}`)
  assert.equal(await page.getByRole('region', { name: 'Repository README' }).evaluate((node) => node.getBoundingClientRect().top), 60)

  await segments.nth(1).hover()
  const hovered = await shownList(page)
  assert.deepEqual(hovered.bold, ['TypeScript28.4%'])
  assert.deepEqual(hovered.rows, ['Rust59.5%', 'TypeScript28.4%', 'JavaScript8.5%', 'HTML1.0%', 'SQL1.0%', 'Python0.7%', 'Other0.9%'])
  await screenshot(page, 'hover.light')
  await segments.nth(6).hover({ force: true })
  await page.locator('[data-slot="tooltip-content"] li.font-semibold', { hasText: 'Other' }).waitFor()
  const list = await page.locator('[data-slot="tooltip-content"]').boundingBox()
  assert.ok(list.x + list.width <= 1280, JSON.stringify(list))
  await page.mouse.move(640, 600)
  await page.locator('[data-slot="tooltip-content"]').waitFor({ state: 'detached' })

  await page.keyboard.press('Tab')
  assert.equal(await first.evaluate((node) => node === document.activeElement), true)
  assert.deepEqual((await shownList(page)).bold, ['Rust59.5%'])
  await page.keyboard.press('Tab')
  assert.equal(await page.locator('main button:focus').count(), 0)
  assert.deepEqual(errors, [])

  await page.goto(`${base}?dark`)
  await segments.nth(0).hover()
  await shownList(page)
  await screenshot(page, 'hover.dark')
  await page.goto(`${base}?dark&dim`)
  const dim = await page.locator('main > div > button').evaluate((node) => getComputedStyle(node).backgroundColor)
  assert.match(dim, /^oklch\(0\.62 /)

  await page.goto(`${base}?empty`)
  await page.getByRole('heading', { name: 'Scope' }).waitFor()
  assert.equal(await page.locator('main button').count(), 0)
  assert.equal(await page.getByRole('region', { name: 'Repository README' }).evaluate((node) => node.getBoundingClientRect().top), 60)
})

test('tapping the language bar on a phone opens the list and tapping elsewhere closes it', async (t) => {
  const { base, page } = await openFixture(t, 'repository-languages', {
    page: { hasTouch: true, isMobile: true, viewport: { width: 390, height: 844 } },
    tailwind: true,
  })
  await page.goto(base)
  const segments = page.locator('main > div > button')
  await segments.first().waitFor()
  assert.deepEqual(await segments.evaluateAll((nodes) => [...new Set(nodes.map((node) => node.getBoundingClientRect().height))]), [4])
  await segments.nth(1).tap()
  assert.deepEqual((await shownList(page)).bold, ['TypeScript28.4%'])
  const list = await page.locator('[data-slot="tooltip-content"]').boundingBox()
  assert.ok(list.x >= 0 && list.x + list.width <= 390, JSON.stringify(list))
  assert.equal(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth), true)
  await screenshot(page, 'tap.phone')
  await page.touchscreen.tap(195, 700)
  await page.locator('[data-slot="tooltip-content"]').waitFor({ state: 'detached' })
})
