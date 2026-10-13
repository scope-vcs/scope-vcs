import assert from 'node:assert/strict'
import test from 'node:test'
import { openFixture } from './support/fixture-server.mjs'

test('a refresh that finds the discussion retained keeps the open composer', async (t) => {
  const { base, errors, page } = await openFixture(t, 'request-discussion-page')
  await page.goto(base, { timeout: 30_000, waitUntil: 'domcontentloaded' })

  await page.getByRole('status').filter({ hasText: 'Loading request discussion' }).waitFor({ state: 'attached' })
  await page.evaluate(() => window.resolveFirstLoad())
  const composer = page.getByRole('textbox', { name: 'Start a new discussion' })
  await composer.fill('Why this file?')
  await composer.evaluate((element) => { window.openComposer = element })

  await page.evaluate(() => window.refreshFromRetained())
  await page.evaluate(() => new Promise((resolve) => requestAnimationFrame(() => requestAnimationFrame(resolve))))
  assert.equal(await composer.inputValue(), 'Why this file?')
  assert.equal(await composer.evaluate((element) => element === window.openComposer), true)
  assert.deepEqual(errors, [])
})
