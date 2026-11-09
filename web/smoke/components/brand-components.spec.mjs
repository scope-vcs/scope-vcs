import assert from 'node:assert/strict'
import { mkdir, writeFile } from 'node:fs/promises'
import test from 'node:test'
import { PNG } from 'pngjs'
import { openFixture } from '../support/fixture-server.mjs'

function ink(image, dark) {
  const png = PNG.sync.read(image)
  const occupied = (x, y) => {
    const offset = (y * png.width + x) * 4
    return dark ? png.data[offset] > 180 : png.data[offset] < 128
  }
  return { ...png, occupied }
}

function center(image, left, right) {
  const rows = []
  for (let y = 0; y < image.height; y++) {
    for (let x = left; x < right; x++) {
      if (image.occupied(x, y)) { rows.push(y); break }
    }
  }
  assert(rows.length, 'The logo must have visible ink')
  return (rows[0] + rows.at(-1)) / 2
}

function visibleSections(image) {
  const seen = new Set()
  let sections = 0
  for (let y = 0; y < image.height; y++) {
    for (let x = 0; x < image.width; x++) {
      const key = y * image.width + x
      if (seen.has(key) || !image.occupied(x, y)) continue
      const queue = [[x, y]]
      seen.add(key)
      for (let i = 0; i < queue.length; i++) {
        const [px, py] = queue[i]
        for (const [nx, ny] of [[px - 1, py], [px + 1, py], [px, py - 1], [px, py + 1]]) {
          const next = ny * image.width + nx
          if (nx < 0 || ny < 0 || nx >= image.width || ny >= image.height || seen.has(next) || !image.occupied(nx, ny)) continue
          seen.add(next)
          queue.push([nx, ny])
        }
      }
      if (queue.length >= 3) sections++
    }
  }
  return sections
}

test('the combined logo stays aligned and contour gaps survive small sizes and blocked fonts', async (t) => {
  const { base, browser, errors, page } = await openFixture(t, 'brand', { tailwind: true })
  const pages = [page, await browser.newPage({ deviceScaleFactor: 2 })]
  for (const [index, current] of pages.entries()) {
    const ratio = index + 1
    await current.goto(base)
    await current.locator('#brand-light img').first().evaluate((image) => image.decode())
    for (const width of [1280, 521, 320]) {
      await current.setViewportSize({ width, height: 500 })
      for (const theme of ['light', 'dark']) {
        const section = current.locator(`#brand-${theme}`)
        const logo = section.locator('header img')
        const pixels = ink(await logo.screenshot(), theme === 'dark')
        const markCenter = center(pixels, 0, 20 * ratio)
        const letterCenter = center(pixels, 27 * ratio, 64 * ratio)
        assert(Math.abs(markCenter - letterCenter) <= 0.5 * ratio, `${theme} ${width}px ${ratio}x: mark and letter body have different centers`)
        const box = await logo.boundingBox()
        assert(Math.abs(box.height - 24) < 0.1 && Math.abs(box.width - 93.3617) < 0.1, 'The approved lockup must keep its size and aspect ratio')
        for (const mark of await section.locator('div img').all()) {
          await mark.evaluate((image) => image.decode())
          assert.equal(visibleSections(ink(await mark.screenshot(), theme === 'dark')), 3, 'Contour cuts must remain open at small icon sizes')
        }
      }
      assert.equal(await current.evaluate(() => document.documentElement.scrollWidth > innerWidth), false)
    }
    const logo = current.locator('#brand-light header img')
    const before = PNG.sync.read(await logo.screenshot())
    await current.route(/\.(woff2?|ttf)(\?.*)?$/, (route) => route.abort())
    await current.reload()
    await logo.evaluate((image) => image.decode())
    const after = PNG.sync.read(await logo.screenshot())
    assert(before.data.equals(after.data), 'Font loading must not change the combined logo')
    if (process.env.SCOPE_COMPONENT_SCREENSHOT) {
      await mkdir(new URL('../../.tmp/brand/', import.meta.url), { recursive: true })
      await writeFile(new URL(`../../.tmp/brand/brand-${ratio}x.png`, import.meta.url), await current.screenshot())
    }
  }
  assert.deepEqual(errors, [])
})
