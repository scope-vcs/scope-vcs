import assert from 'node:assert/strict'
import { test } from 'node:test'
import { waitForClientHydration, withPage } from './browser-smoke.mjs'

const publicLayer = '[data-view="public"]'
const privateLayer = '[data-view="private"]'

async function withLanding(run, pageOptions = {}) {
  await withPage('/', async (page) => {
    await page.getByRole('heading', { name: 'One repository. Part of it is public.' }).waitFor()
    const themeToggle = page.getByRole('button', { name: 'Switch to light mode' })
    await waitForClientHydration(themeToggle)
    await themeToggle.click()
    await page.getByRole('button', { name: 'Switch to dark mode' }).waitFor()
    await run(page)
  }, {
    permissions: ['clipboard-read', 'clipboard-write'],
    viewport: { width: 1440, height: 1000 },
    ...pageOptions,
  })
}

/** Waits until the lens radius, in page pixels, is inside the given bounds. */
async function waitForRadius(page, { above = -Infinity, below = Infinity }) {
  await page.waitForFunction(({ selector, above, below }) => {
    const layer = document.querySelector(selector)
    const zoom = Number(/scale\(([\d.]+)\)/.exec(layer.style.transform)?.[1] ?? 1)
    const radius = Number(/circle\(([\d.]+)px/.exec(layer.style.clipPath)?.[1] ?? 0) * zoom
    return radius > above && radius < below
  }, { selector: privateLayer, above, below })
}

test('landing install controls copy the selected command and keep the theme after reload', async () => {
  await withLanding(async (page) => {
    await page.reload()
    await page.getByRole('button', { name: 'Switch to dark mode' }).waitFor()
    assert.equal(await page.locator('html').getAttribute('class'), '')
    await page.getByRole('link', { name: 'Install Scope', exact: true }).click()
    await page.locator(`${publicLayer} .landing-terminal.is-called`).waitFor()
    await page.locator(`${publicLayer} .landing-terminal.is-called`).waitFor({ state: 'detached' })
    for (const [platform, copyName, script] of [
      ['Windows', 'Windows', 'install.ps1'],
      ['macOS / Linux', 'macOS and Linux', 'install.sh'],
    ]) {
      const option = page.getByRole('button', { name: platform, exact: true })
      await option.click()
      assert.equal(await option.getAttribute('aria-pressed'), 'true')
      const command = await page.locator(`${publicLayer} [data-note="command"] code`).innerText()
      assert(command.includes(script))
      assert.equal(await page.locator(`${privateLayer} [data-note="command"] code`).innerText(), `${command}  # yes, we know`)
      await page.getByRole('button', { name: `Copy ${copyName} install command` }).click()
      assert.equal(await page.evaluate(() => navigator.clipboard.readText()), command)
    }
    assert.match(await page.getByRole('link', { name: 'Sign in', exact: true }).getAttribute('href'), /^\/sign-in/)
    assert.equal(await page.getByRole('link', { name: 'Licenses', exact: true }).getAttribute('href'), '/licenses')
  })
})

test('landing layout keeps every note clear of the content at each width', async () => {
  await withLanding(async (page) => {
    for (const colorScheme of ['light', 'dark']) {
      if (colorScheme === 'dark') await page.getByRole('button', { name: 'Switch to dark mode' }).click()
      for (const width of [1920, 1440, 1024, 900, 768, 390, 320]) {
        await page.setViewportSize({ width, height: 1000 })
        const layout = await page.evaluate((selector) => {
          const layer = document.querySelector(selector)
          const notes = [...layer.querySelectorAll('[data-note]:not([data-note="command"])')].filter((note) => note.getClientRects().length)
          const content = [...layer.querySelectorAll('h1, h2, p:not(.landing-note), a, button, .repo-panel, .merge-graph, [data-note="command"]')]
          const overlaps = (a, b) => a.left < b.right - 1 && b.left < a.right - 1 && a.top < b.bottom - 1 && b.top < a.bottom - 1
          return {
            notes: notes.length,
            collisions: notes.flatMap((note) => [...content, ...notes]
              .filter((other) => other !== note && !note.contains(other) && !other.contains(note) && overlaps(note.getBoundingClientRect(), other.getBoundingClientRect()))
              .map((other) => `${note.dataset.note} overlaps ${other.dataset.note ?? other.tagName}`)),
            overflow: document.documentElement.scrollWidth > innerWidth,
            tops: innerWidth > 900 ? [['merge', 'install'].map((id) => document.querySelector(`#${id} h2`).getBoundingClientRect().top)] : [],
          }
        }, publicLayer)
        assert.equal(layout.notes, 8, `${colorScheme} at ${width}px`)
        assert.deepEqual(layout.collisions, [], `${colorScheme} at ${width}px`)
        assert.equal(layout.overflow, false, `${colorScheme} at ${width}px`)
        for (const [merge, install] of layout.tops) assert.equal(merge, install, `${colorScheme} at ${width}px`)
      }
    }
  })
})

test('the lens follows the pointer, closes over links, floods on hold and goes away with L', async () => {
  await withLanding(async (page) => {
    const layer = page.locator(privateLayer)
    assert.equal(await layer.getAttribute('aria-hidden'), 'true')
    assert.equal(await layer.evaluate((element) => element.inert), true)
    assert.equal(await page.getByRole('heading', { level: 1 }).count(), 1)

    await page.mouse.move(700, 300)
    await waitForRadius(page, { above: 150 })
    const install = await page.getByRole('link', { name: 'Install Scope', exact: true }).boundingBox()
    await page.mouse.move(install.x + install.width / 2, install.y + install.height / 2)
    await waitForRadius(page, { below: 1 })

    await page.mouse.move(700, 300)
    await page.mouse.down()
    await waitForRadius(page, { above: Math.hypot(1440, 1000) })
    await page.mouse.up()
    await waitForRadius(page, { below: 200 })

    await page.keyboard.press('l')
    await waitForRadius(page, { below: 1 })
    assert.equal(await page.locator('.landing').evaluate((element) => getComputedStyle(element).cursor), 'auto')
    await page.getByRole('status').filter({ hasText: 'press L to bring it back' }).waitFor()
    await page.keyboard.press('l')
    await waitForRadius(page, { above: 150 })
  })
})

for (const reducedMotion of ['no-preference', 'reduce']) {
  test(`finding every note fires confetti (${reducedMotion} motion)`, async () => {
    await withLanding(async (page) => {
      const notes = page.locator(`${publicLayer} [data-note]`)
      const count = await notes.count()
      assert.equal(count, 9)
      for (let index = 0; index < count; index++) {
        const note = notes.nth(index)
        await note.scrollIntoViewIfNeeded()
        const box = await note.boundingBox()
        await page.mouse.move(box.x + box.width / 2, box.y + box.height / 2)
        await page.waitForTimeout(reducedMotion === 'reduce' ? 250 : 900)
      }
      await page.locator('canvas').waitFor({ state: 'attached' })
    }, { reducedMotion })
  })
}

test('without JavaScript the landing page keeps the native cursor', async () => {
  await withPage('/', async (page) => {
    await page.getByRole('heading', { name: 'One repository. Part of it is public.' }).waitFor()
    assert.notEqual(await page.locator('.landing').evaluate((element) => getComputedStyle(element).cursor), 'none')
  }, { javaScriptEnabled: false, viewport: { width: 1440, height: 1000 } })
})

test('on touch screens only the lens rim catches touches', async () => {
  await withLanding(async (page) => {
    // Playwright's click() moves an emulated mouse onto the theme toggle, where the lens closes.
    await page.mouse.move(200, 700)
    await waitForRadius(page, { above: 100 })
    const hits = await page.locator(privateLayer).evaluate((layer) => {
      const [, radius, x, y] = /circle\(([\d.]+)px at ([\d.-]+)px ([\d.-]+)px\)/.exec(layer.style.clipPath).map(Number)
      const zoom = Number(/scale\(([\d.]+)\)/.exec(layer.style.transform)?.[1] ?? 1)
      const box = layer.parentElement.getBoundingClientRect()
      const hit = (dx) => document.elementFromPoint(box.left + x + dx, box.top + y)?.classList.contains('lens-grip') ?? false
      return { center: hit(0), rim: hit(radius * zoom) }
    })
    assert.deepEqual(hits, { center: false, rim: true })
  }, { hasTouch: true, isMobile: true, viewport: { width: 390, height: 844 } })
})

test('install platform controls retain touch-sized targets on tablets', async () => {
  await withLanding(async (page) => {
    await page.setViewportSize({ width: 768, height: 1000 })
    assert.equal(await page.evaluate(() => matchMedia('(pointer: coarse)').matches), true)
    const controls = page.getByRole('group', { name: 'Operating system' }).getByRole('button')
    assert.equal(await controls.count(), 2)
    for (const control of await controls.all()) {
      assert((await control.boundingBox()).height >= 44)
    }
  }, { hasTouch: true })
})
