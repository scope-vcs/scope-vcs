import assert from 'node:assert/strict'
import { test } from 'node:test'
import {
  repoPath,
  waitForClientHydration,
  withPage,
} from './browser-smoke.mjs'

const tabletViewport = { width: 1024, height: 768 }

test('workspace tabs keep touch close controls visible and large enough', async () => {
  await withPage(repoPath, async (page) => {
    const firstTab = page.getByRole('tab').first()
    await firstTab.waitFor()
    await waitForClientHydration(firstTab)
    await firstTab.dblclick()
    const firstLabel = await firstTab.getAttribute('aria-label')
    assert(firstLabel)

    await page.evaluate((to) => {
      void globalThis.__TSR_ROUTER__.navigate({
        to,
        search: { file: 'src/app.ts' },
      })
    }, repoPath)
    await page.getByRole('tab', { name: 'src/app.ts', exact: true }).waitFor()
    const close = page.getByTitle(`Close ${firstLabel}`, { exact: true })
    await page.waitForFunction(
      (element) => getComputedStyle(element).opacity === '1',
      await close.elementHandle(),
    )
    const presentation = await close.evaluate((element) => {
      const bounds = element.getBoundingClientRect()
      return {
        height: bounds.height,
        opacity: getComputedStyle(element).opacity,
        width: bounds.width,
      }
    })
    assert.equal(presentation.opacity, '1')
    assert(presentation.height >= 44, `close target was ${presentation.height}px high`)
    assert(presentation.width >= 44, `close target was ${presentation.width}px wide`)

    await close.click()
    assert.equal(await page.getByRole('tab', { name: firstLabel, exact: true }).count(), 0)
  }, {
    hasTouch: true,
    viewport: tabletViewport,
  })
})

test('workspace tabs retain mouse hover disclosure', async () => {
  await withPage(repoPath, async (page) => {
    const firstTab = page.getByRole('tab').first()
    await firstTab.waitFor()
    await waitForClientHydration(firstTab)
    await firstTab.dblclick()
    const firstLabel = await firstTab.getAttribute('aria-label')
    assert(firstLabel)

    await page.evaluate((to) => {
      void globalThis.__TSR_ROUTER__.navigate({
        to,
        search: { file: 'src/app.ts' },
      })
    }, repoPath)
    await page.getByRole('tab', { name: 'src/app.ts', exact: true }).waitFor()
    await page.mouse.move(tabletViewport.width - 1, tabletViewport.height - 1)
    const close = page.getByTitle(`Close ${firstLabel}`, { exact: true })
    await page.waitForFunction(
      (element) => getComputedStyle(element).opacity === '0',
      await close.elementHandle(),
    )
    await firstTab.hover()
    await page.waitForFunction(
      (element) => getComputedStyle(element).opacity === '1',
      await close.elementHandle(),
    )
  }, { viewport: tabletViewport })
})

test('workspace tabs close from the keyboard and an accessible control outside the tablist', async () => {
  await withPage(repoPath, async (page) => {
    const firstTab = page.getByRole('tab').first()
    await firstTab.waitFor()
    await waitForClientHydration(firstTab)
    await firstTab.dblclick()
    const firstLabel = await firstTab.getAttribute('aria-label')
    assert(firstLabel)

    await page.evaluate((to) => {
      void globalThis.__TSR_ROUTER__.navigate({
        to,
        search: { file: 'src/app.ts' },
      })
    }, repoPath)
    const secondTab = page.getByRole('tab', { name: 'src/app.ts', exact: true })
    await secondTab.waitFor()
    assert.equal(await page.getByRole('tablist').getByRole('button').count(), 0)

    await firstTab.focus()
    await page.keyboard.press('Delete')
    await page.getByRole('tab', { name: firstLabel, exact: true }).waitFor({ state: 'detached' })
    assert.equal(await secondTab.evaluate((element) => element === document.activeElement), true)

    // Visually hidden until focused; assistive tech activates it without a pointer.
    await page.getByRole('button', { name: 'Close src/app.ts', exact: true }).focus()
    await page.keyboard.press('Enter')
    await secondTab.waitFor({ state: 'detached' })
  }, { viewport: tabletViewport })
})
