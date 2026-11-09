import assert from 'node:assert/strict'
import { readFile, writeFile, mkdir } from 'node:fs/promises'
import { chromium } from 'playwright'
import { PNG } from 'pngjs'

const check = process.argv.includes('--check')
const web = new URL('../', import.meta.url)
const output = new URL('public/brand/contour-v1/', web)
const master = await readFile(new URL('brand/scope.svg', web), 'utf8')
const browser = await chromium.launch({ headless: true, args: ['--disable-lcd-text'] })

try {
  const page = await browser.newPage({ deviceScaleFactor: 1 })
  const artwork = await page.evaluate((source) => {
    const document = new DOMParser().parseFromString(source.replace(/>\s+</g, '><'), 'image/svg+xml')
    if (document.querySelector('parsererror')) throw new Error('Invalid brand master SVG')
    const root = document.documentElement
    const mark = document.getElementById('scope-mark')
    const wordmark = document.getElementById('scope-wordmark')
    if (!mark || !wordmark) throw new Error('Brand master needs scope-mark and scope-wordmark groups')
    const serialize = (element) => new XMLSerializer().serializeToString(element)
    const markBody = [...mark.children].map(serialize).join('')
    return {
      lockup: serialize(root),
      markBody,
      lockupBody: [...root.children].map(serialize).join(''),
      width: Number(root.getAttribute('width')),
      height: Number(root.getAttribute('height')),
    }
  }, master)
  const svg = (width, height, content) => `<svg xmlns="http://www.w3.org/2000/svg" width="${width}" height="${height}" viewBox="0 0 ${width} ${height}">${content}</svg>\n`
  const mark = svg(24, 24, artwork.markBody)
  const iconBody = `<rect width="32" height="32" rx="6" fill="#24231f"/><g color="#f4f2ec" transform="translate(3 3) scale(${26 / 24})">${artwork.markBody}</g>`
  await mkdir(output, { recursive: true })

  async function save(name, bytes) {
    const destination = new URL(name, output)
    if (!check) return writeFile(destination, bytes)
    const committed = await readFile(destination)
    if (!name.endsWith('.png')) {
      assert.equal(committed.toString(), bytes.toString(), `${name} differs from the brand master`)
      return
    }
    const actual = PNG.sync.read(bytes)
    const expected = PNG.sync.read(committed)
    assert.deepEqual([actual.width, actual.height], [expected.width, expected.height], `${name} dimensions changed`)
    assert(actual.data.equals(expected.data), `${name} pixels differ from the brand master`)
  }

  async function raster(name, source, width, height) {
    await page.setViewportSize({ width, height })
    await page.setContent(`<html><body style="margin:0;color:#24231f;background:transparent">${source}</body></html>`)
    const bytes = await page.locator('svg').first().screenshot({ omitBackground: true })
    await save(name, bytes)
  }

  await save('scope-lockup.svg', artwork.lockup + '\n')
  await save('scope-mark.svg', mark)
  await save('favicon.svg', svg(32, 32, iconBody))
  for (const [name, size] of [['favicon.png', 32], ['apple-touch-icon.png', 180], ['scope-avatar.png', 512]]) {
    const scaled = svg(size, size, `<g transform="scale(${size / 32})">${iconBody}</g>`)
    await raster(name, scaled, size, size)
  }
  const social = svg(1200, 630, `<rect width="1200" height="630" fill="#24231f"/><g color="#f4f2ec" transform="translate(${(1200 - artwork.width * 6) / 2} ${(630 - artwork.height * 6) / 2}) scale(6)">${artwork.lockupBody}</g>`)
  await raster('scope-social.png', social, 1200, 630)
  const email = svg(188, 48, `<rect width="188" height="48" fill="#f4f2ec"/><g transform="translate(${94 - artwork.width} 0) scale(2)">${artwork.lockupBody}</g>`)
  await raster('scope-email.png', email, 188, 48)

  const readme = new URL('../README.html', web)
  const html = await readFile(readme, 'utf8')
  const symbol = (id, viewBox, body) => `<symbol id="${id}" viewBox="${viewBox}">${body.replaceAll('scope-', `repository-${id}-`)}</symbol>`
  const updated = html
    .replace(/<symbol id="mark"[\s\S]*?<\/symbol>/, symbol('mark', '0 0 24 24', artwork.markBody))
    .replace(/<symbol id="logo"[\s\S]*?<\/symbol>/, symbol('logo', `0 0 ${artwork.width} ${artwork.height}`, artwork.lockupBody))
  if (check) assert.equal(html, updated, 'README.html contains stale brand symbols')
  else await writeFile(readme, updated)
  console.log(`${check ? 'Checked' : 'Generated'} Scope brand exports and repository homepage symbols`)
} finally {
  await browser.close()
}
