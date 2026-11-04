import assert from 'node:assert/strict'
import { test } from 'node:test'
import {
  assertCurrentRepoSection,
  repoPath,
  requestRepoPath,
  waitForClientHydration,
  withPage,
} from './browser-smoke.mjs'

test('public repository requests route is anonymously readable', async () => {
  await withPage(`${repoPath}/requests`, async (page) => {
    await assertCurrentRepoSection(page, 'Requests')
    await page.getByRole('complementary', { name: 'Requests workspace' }).waitFor()
    assert.equal(await page.getByRole('heading', { level: 2, name: /^your work$/i }).count(), 0)
    await page.getByText('You’re caught up.', { exact: true }).waitFor()
    assert.equal(await page.getByRole('button', { name: /Set aside/ }).count(), 0)
    assert.equal(await page.getByRole('button', { name: /Unclaimed/ }).count(), 0)
    assert.equal(await page.getByRole('button', { name: /Done/ }).count(), 0)
  })
})

test('request queue rows are keyboard reachable and mobile rows do not overflow', async () => {
  await withPage(
    `${requestRepoPath}/requests`,
    async (page) => {
      const readyRow = page.getByRole('link', {
        name: /Add bounded retry timing/,
      })
      await readyRow.waitFor()
      await page.getByRole('link', { name: 'Requests', exact: true }).getByText(/^\d+$/).waitFor()
      await readyRow.focus()
      assert.equal(
        await readyRow.evaluate(
          (element) => element === document.activeElement,
        ),
        true,
      )
      assert.equal(
        await page.evaluate(
          () =>
            document.documentElement.scrollWidth <=
            document.documentElement.clientWidth,
        ),
        true,
      )
    },
    { viewport: { height: 844, width: 390 } },
  )
})

test('requests sidebar resizes, collapses, and pins again by dragging or keyboard', async () => {
  await withPage(`${requestRepoPath}/requests/req_demo_ready`, async (page) => {
    const sidebar = page.getByRole('complementary', { name: 'Requests workspace' })
    const separator = page.getByRole('separator', { name: 'Requests sidebar width' })
    await sidebar.getByRole('link', { name: /Add bounded retry timing/ }).waitFor()
    await waitForClientHydration(separator)
    const width = () => sidebar.evaluate((element) => element.getBoundingClientRect().width)
    const settles = (min, max = min) => page.waitForFunction(([low, high]) => {
      const { width } = document.querySelector('.request-workspace-sidebar').getBoundingClientRect()
      return width >= low && width <= high
    }, [min, max])
    async function dragBy(distance) {
      const bounds = await separator.boundingBox()
      assert(bounds)
      const x = bounds.x + bounds.width / 2
      const y = Math.max(bounds.y + 24, 110)
      await page.mouse.move(x, y)
      await page.mouse.down()
      await page.mouse.move(x + distance, y, { steps: 8 })
      await page.mouse.up()
    }
    const originalWidth = await width()
    await dragBy(-90)
    await settles(originalWidth - 90)
    await sidebar.getByRole('button', { name: 'Collapse requests sidebar' }).click()
    assert.equal(await separator.getAttribute('aria-valuetext'), 'Collapsed')
    await sidebar.getByRole('button', { name: 'Expand requests sidebar' }).click()
    assert.notEqual(await separator.getAttribute('aria-valuetext'), 'Collapsed')
    await settles(originalWidth - 90)
    await dragBy(-180)
    assert.equal(await separator.getAttribute('aria-valuetext'), 'Collapsed')
    await dragBy(180)
    await settles(180, 10_000)
    assert.notEqual(await separator.getAttribute('aria-valuetext'), 'Collapsed')
    await separator.focus()
    await page.keyboard.press('Home')
    await settles(180)
    await page.keyboard.press('ArrowLeft')
    assert.equal(await separator.getAttribute('aria-valuetext'), 'Collapsed')
    await page.keyboard.press('ArrowRight')
    await settles(180)
    await page.keyboard.press('End')
    await settles(360)
    await page.setViewportSize({ width: 390, height: 844 })
    assert.equal(await separator.isVisible(), false)
  })
})
