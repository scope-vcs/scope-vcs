import assert from 'node:assert/strict'
import test from 'node:test'
import { openFixture } from '../support/fixture-server.mjs'

const screenshot = async (page, name) => {
  if (process.env.SCOPE_COMPONENT_SCREENSHOT) {
    await page.screenshot({ path: `${process.env.SCOPE_COMPONENT_SCREENSHOT}-${name}.png`, fullPage: true })
  }
}

test('update changes list code and visibility changes once, filtered, grouped, and stepped', async (t) => {
  const { base, errors, page } = await openFixture(t, 'history-entry-changes', { tailwind: true })
  await page.goto(base, { timeout: 30_000, waitUntil: 'domcontentloaded' })
  const filters = page.getByRole('radiogroup', { name: 'Show changes' })
  const list = page.getByRole('list', { name: 'Changed paths' })
  const diff = (path) => page.getByLabel(`${path} diff`, { exact: true })

  await filters.getByRole('radio', { name: 'All 354' }).waitFor({ timeout: 30_000 })
  assert.deepEqual(await filters.getByRole('radio').allTextContents(), [
    'All354', 'Visibility143', 'Added117', 'Modified179', 'Deleted57',
  ])
  const api = list.getByRole('button', { name: 'api/, 43 files, 34 visibility changes', exact: true })
  assert.equal(await api.getAttribute('aria-expanded'), 'false')
  assert.equal(await list.getByRole('button', { name: 'web/, 131 files', exact: true }).count(), 1)
  assert.equal(await list.getByRole('button', { name: /^README\.md/ }).count(), 1)
  await screenshot(page, 'collapsed')

  await filters.getByRole('radio', { name: /Visibility 143$/ }).click()
  await list.getByRole('button', { name: 'bench/, 11 files', exact: true }).waitFor()
  assert.equal(await list.getByRole('button', { name: /^web\// }).count(), 0)
  await list.getByRole('button', { name: /^api\// }).click()
  await list.getByText('src/git/', { exact: true }).waitFor()
  await list.getByRole('button', { name: 'file_00.rs Public visibility', exact: true }).first().click()
  await diff('api/src/git/file_00.rs').getByText('code diff /api/src/git/file_00.rs').waitFor()
  await page.getByText('1 of 142', { exact: true }).waitFor()
  await page.keyboard.press('j')
  await diff('api/src/git/file_02.rs').waitFor()
  await page.getByText('2 of 142', { exact: true }).waitFor()
  await page.keyboard.press('k')
  await diff('api/src/git/file_00.rs').waitFor()
  assert.equal(await page.getByRole('button', { name: 'Previous changed file' }).isDisabled(), true)
  await page.getByRole('button', { name: 'Next changed file' }).click()
  await diff('api/src/git/file_02.rs').waitFor()
  assert.equal(await list.locator('[aria-current="true"]').getAttribute('title'), 'api/src/git/file_02.rs · Modified')
  await screenshot(page, 'visibility-filter')

  await filters.getByRole('radio', { name: 'All 354' }).click()
  await page.getByRole('searchbox', { name: 'Filter changed paths' }).fill('http/file_00')
  assert.deepEqual(
    await list.getByRole('button').evaluateAll((buttons) => buttons.map((button) => button.getAttribute('aria-label') ?? button.textContent)),
    ['api/, 1 file, 1 visibility change', 'Mfile_00.rs'],
  )
  await list.getByRole('button', { name: 'file_00.rs Private visibility', exact: true }).click()
  const context = diff('api/src/http/file_00.rs')
  await context.getByText('· was Private', { exact: true }).waitFor()
  await context.getByText('· was Public', { exact: true }).waitFor()
  const targets = context.getByRole('radiogroup', { name: 'Diff to show' })
  assert.deepEqual(await targets.getByRole('radio').allTextContents(), ['Code change', 'Moved to Public preview'])
  await targets.getByRole('radio', { name: 'Moved to Public preview' }).click()
  await context.getByText('visibility preview set:/api/src/http/file_00.rs').waitFor()
  assert.deepEqual(await page.evaluate(() => window.selection), {
    path: '/api/src/http/file_00.rs', visibilityId: 'set:/api/src/http/file_00.rs',
  })
  await targets.getByRole('radio', { name: 'Code change' }).click()
  await context.getByText('code diff /api/src/http/file_00.rs').waitFor()
  await screenshot(page, 'visibility-context')

  await page.getByRole('searchbox', { name: 'Filter changed paths' }).fill('policy')
  const visibilityOnly = list.getByTitle('docs/policy.md · Visibility only')
  await visibilityOnly.waitFor()
  assert.equal(await visibilityOnly.evaluate((element) => element.tagName), 'DIV')
  await visibilityOnly.getByText('Moved to Public', { exact: true }).waitFor()
  await visibilityOnly.getByText('· was Private', { exact: true }).waitFor()
  await screenshot(page, 'visibility-only')
  await page.getByRole('searchbox', { name: 'Filter changed paths' }).fill('nothing-matches')
  await page.getByText('No changed paths match.', { exact: true }).waitFor()

  await page.getByRole('button', { name: 'Clear path filter' }).click()
  await page.setViewportSize({ width: 390, height: 844 })
  await list.getByRole('button', { name: 'file_02.rs Public visibility', exact: true }).first().click()
  assert.equal(await list.isVisible(), false)
  await page.getByRole('searchbox', { name: 'Filter changed paths' }).fill('policy')
  await visibilityOnly.waitFor()
  await page.getByRole('button', { name: 'Clear path filter' }).click()
  await page.getByRole('button', { name: 'Expand all' }).click()
  assert.equal(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth), true)
  await screenshot(page, 'mobile')
  assert.deepEqual(errors, [])
})
