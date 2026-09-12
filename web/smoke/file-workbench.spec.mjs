import assert from 'node:assert/strict'
import { test } from 'node:test'
import {
  assertMobileFilesCollapsed,
  assertNoHorizontalOverflow,
  baseUrl,
  repoPath,
  waitForClientHydration,
  withPage,
} from './browser-smoke.mjs'

const viewport = { width: 1280, height: 900 }

async function readyDiff(page, path) {
  const diff = page.getByLabel(`${path} diff`, { exact: true })
  await diff.waitFor()
  await diff.locator('[data-slot="pending-surface"]').waitFor({ state: 'detached' })
  return diff
}

test('file workbench resizes with pointer and keyboard and keeps mobile navigation available', async () => {
  await withPage(repoPath, async (page) => {
    await page.getByRole('tab', { name: 'README.html' }).waitFor()
    const separator = page.getByRole('separator', { name: 'File pane width' })
    await separator.waitFor()
    await waitForClientHydration(separator)
    assert.equal(await separator.getAttribute('aria-valuenow'), '250')
    await separator.focus()
    await page.keyboard.press('End')
    assert.equal(await separator.getAttribute('aria-valuenow'), '360')
    await page.keyboard.press('Home')
    assert.equal(await separator.getAttribute('aria-valuenow'), '180')
    await page.keyboard.press('ArrowRight')
    assert.equal(await separator.getAttribute('aria-valuenow'), '190')
    const bounds = await separator.boundingBox()
    await page.mouse.move(bounds.x, bounds.y + 50)
    await page.mouse.down()
    await page.mouse.move(bounds.x + 600, bounds.y + 50)
    await page.mouse.up()
    assert.equal(await separator.getAttribute('aria-valuenow'), '360')
    await page.setViewportSize({ width: 390, height: 844 })
    const toggle = page.getByRole('button', { name: /^files README.html$/ })
    await toggle.waitFor()
    assert.equal(await toggle.getAttribute('aria-expanded'), 'false')
    await toggle.click()
    await page.getByRole('button', { name: 'Expand src' }).click()
    await page.getByRole('button', { name: 'app.ts', exact: true }).click()
    await assertMobileFilesCollapsed(page, 'src/app.ts')
    await page.getByRole('button', { name: 'files src/app.ts', exact: true }).click()
    await page.getByRole('button', { name: 'README.html', exact: true }).waitFor()
    await assertNoHorizontalOverflow(page)
  }, { viewport })
})

test('history default selection respects explicit links, close, reselect and browser Back', async () => {
  await withPage(`${repoPath}/history`, async (page) => {
    await readyDiff(page, 'README.html')
    assert.equal(new URL(page.url()).searchParams.has('path'), false)
    await page.getByRole('button', { name: 'Close diff viewer' }).click()
    await page.getByText('Select a changed file', { exact: true }).waitFor()
    await page.getByRole('button', { name: 'README.html', exact: true }).click()
    await readyDiff(page, 'README.html')
    await page.goto(`${baseUrl}${repoPath}/history?entry=dev-public-1&path=%2Fsrc%2Fapp.ts`)
    await readyDiff(page, 'src/app.ts')
    await page.goBack()
    await readyDiff(page, 'README.html')
    await page.evaluate((to) => globalThis.__TSR_ROUTER__.navigate({
      to, search: { entry: 'dev-public-1', path: '/src/app.ts' },
    }), `${repoPath}/history`)
    await readyDiff(page, 'src/app.ts')
    await page.getByRole('button', { name: 'Close diff viewer' }).click()
    await page.getByText('Select a changed file', { exact: true }).waitFor()
    await page.evaluate((to) => globalThis.__TSR_ROUTER__.navigate({
      to, search: { entry: 'dev-public-1', path: '/README.html' },
    }), `${repoPath}/history`)
    await readyDiff(page, 'README.html')
    await page.goBack()
    await readyDiff(page, 'src/app.ts')
    await page.getByRole('button', { name: 'README.html', exact: true }).click()
    await readyDiff(page, 'README.html')
    await page.setViewportSize({ width: 320, height: 800 })
    const toggle = page.getByRole('button', { name: /^files README.html$/ })
    await toggle.click()
    const expandSource = page.getByRole('button', { name: 'Expand src' })
    if (await expandSource.count()) await expandSource.click()
    await page.getByRole('button', { name: 'app.ts', exact: true }).click()
    await readyDiff(page, 'src/app.ts')
    await assertMobileFilesCollapsed(page, 'src/app.ts')
  }, { viewport })
})
