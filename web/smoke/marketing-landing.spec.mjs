import assert from 'node:assert/strict'
import { test } from 'node:test'
import { chromium } from 'playwright'

const baseUrl = process.env.SCOPE_WEB_BASE_URL ?? process.env.PLAYWRIGHT_BASE_URL ?? 'http://localhost:3000'

async function withPage(run) {
  const browser = await chromium.launch({ headless: true })
  const context = await browser.newContext({
    permissions: ['clipboard-read', 'clipboard-write'],
    viewport: { width: 1440, height: 1000 },
  })
  const page = await context.newPage()
  const errors = []
  page.on('pageerror', (error) => errors.push(error.message))
  try {
    await page.goto(baseUrl)
    await page.getByRole('heading', { name: 'One repository. You choose what’s public.' }).waitFor()
    await page.getByRole('button', { name: 'Switch to light mode' }).click()
    await page.getByRole('button', { name: 'Switch to dark mode' }).waitFor()
    await run(page)
    assert.deepEqual(errors, [])
  } finally {
    await browser.close()
  }
}

test('landing install controls copy the selected command and keep the theme after reload', async () => {
  await withPage(async (page) => {
    await page.reload()
    await page.getByRole('button', { name: 'Switch to dark mode' }).waitFor()
    assert.equal(await page.locator('html').getAttribute('class'), '')
    await page.getByRole('link', { name: 'Install Scope', exact: true }).click()
    assert.equal(new URL(page.url()).hash, '#install')
    for (const [platform, copyName, script] of [
      ['Windows', 'Windows', 'install.ps1'],
      ['macOS / Linux', 'macOS and Linux', 'install.sh'],
    ]) {
      const option = page.getByRole('button', { name: platform, exact: true })
      await option.click()
      assert.equal(await option.getAttribute('aria-pressed'), 'true')
      const command = await page.locator('.terminal code').innerText()
      assert(command.includes(script))
      await page.getByRole('button', { name: `Copy ${copyName} install command` }).click()
      assert.equal(await page.evaluate(() => navigator.clipboard.readText()), command)
      assert.equal(await page.locator('details').getAttribute('open'), '')
      await page.getByText('Already installed?', { exact: true }).click()
      assert.equal(await page.locator('details').getAttribute('open'), null)
    }
    assert.match(await page.getByRole('link', { name: 'Sign in', exact: true }).getAttribute('href'), /^\/sign-in/)
    assert.equal(await page.getByRole('link', { name: 'Licenses', exact: true }).getAttribute('href'), '/licenses')
  })
})

test('landing visuals stay aligned and loop through public sharing, review and merge', async () => {
  await withPage(async (page) => {
    for (const colorScheme of ['light', 'dark']) {
      if (colorScheme === 'dark') await page.getByRole('button', { name: 'Switch to dark mode' }).click()
      for (const width of [1920, 1440, 1024, 900, 768, 390, 320]) {
        await page.setViewportSize({ width, height: 1000 })
        const layout = await page.evaluate(() => {
          const boxes = ['.repository', '.request-progress', '.request-sheet', '.terminal']
            .map((selector) => document.querySelector(selector).getBoundingClientRect())
          const size = (selector) => getComputedStyle(document.querySelector(selector)).fontSize
          return {
            edges: boxes.map(({ left, right }) => [left, right]),
            overflow: document.documentElement.scrollWidth > innerWidth,
            fonts: [size('.repository-file'), size('.scene-file'), size('.review-code')],
            dividers: ['.topbar', '.contributions', '.install', '.footer'].map((selector) => {
              const style = getComputedStyle(document.querySelector(selector))
              return [style.borderTopWidth, style.borderBottomWidth]
            }),
          }
        })
        for (const edges of layout.edges) assert.deepEqual(edges, layout.edges[0], `${colorScheme} at ${width}px`)
        assert.equal(layout.overflow, false)
        assert.equal(new Set(layout.fonts).size, 1)
        assert(layout.dividers.every(([top, bottom]) => top === '0px' && bottom === '0px'))
      }
    }
    // The illustrations are passive timelines, including with the app's reduced-motion preference.
    for (const reducedMotion of ['no-preference', 'reduce']) {
      await page.emulateMedia({ reducedMotion })
      const frames = await page.evaluate(() => {
        function seek(selector, time) {
          const animations = document.querySelector(selector).getAnimations({ subtree: true })
          for (const animation of animations) {
            animation.pause()
            animation.currentTime = time
          }
          return animations.length
        }
        const opacity = (selector) => Number(getComputedStyle(document.querySelector(selector)).opacity)
        const sharing = [0, 4000, 8000].map((time) => {
          const count = seek('.repository', time)
          return { count, public: opacity('.shared-example'), private: opacity('.source-example .is-private') }
        })
        const requests = [0, 6500, 11500, 14000].map((time) => {
          const count = seek('.contribution-flow', time)
          return { count, scenes: ['.submission-scene', '.review-scene', '.merged-scene'].map(opacity) }
        })
        seek('.contribution-flow', 9300)
        const comment = document.querySelector('.maintainer-review').getBoundingClientRect()
        const decision = document.querySelector('.review-decision').getBoundingClientRect()
        return { sharing, requests, commentFits: comment.bottom <= decision.top }
      })
      assert(frames.sharing.every(({ count }) => count > 0))
      assert.deepEqual(frames.sharing.map(({ public: shared }) => shared), [0, 1, 0])
      assert.deepEqual(frames.sharing.map(({ private: hidden }) => hidden), [1, 0, 1])
      assert(frames.requests.every(({ count }) => count > 0))
      assert.deepEqual(frames.requests.map(({ scenes }) => scenes), [[1, 0, 0], [0, 1, 0], [0, 0, 1], [1, 0, 0]])
      assert.equal(frames.commentFits, true)
    }
  })
})
