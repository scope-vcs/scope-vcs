import assert from 'node:assert/strict'
import test from 'node:test'
import { openFixture } from './support/fixture-server.mjs'

test('auto-merge confirms one revision and stays usable on desktop and mobile', async (t) => {
  const { base, errors, page } = await openFixture(t, 'request-auto-merge', { tailwind: true })
  await page.goto(base, { timeout: 30_000, waitUntil: 'domcontentloaded' })

  await page.getByRole('button', { name: 'Enable auto-merge' }).click()
  const dialog = page.getByRole('alertdialog')
  await dialog.getByText('a'.repeat(40), { exact: true }).waitFor()
  assert.equal(await dialog.getByText('revision-7d9f2f1b', { exact: true }).count(), 0)
  await page.evaluate(() => window.refreshAutoMerge())
  await dialog.getByText('a'.repeat(40), { exact: true }).waitFor()
  await dialog.getByRole('button', { name: 'Enable auto-merge' }).click()
  await page.getByText('Auto-merge enabled', { exact: true }).waitFor()
  assert.equal(
    await page.getByText(/Authorized by you/).textContent(),
    'Authorized by you · aaaaaaaaaaaa · CI is still running.',
  )
  assert.deepEqual(await page.evaluate(() => window.calls[0]), {
    expected_head_oid: 'a'.repeat(40),
    expected_revision_id: 'revision-7d9f2f1b',
  })

  await page.setViewportSize({ width: 390, height: 844 })
  assert.equal(await page.evaluate(() => document.documentElement.scrollWidth > innerWidth), false)
  const footerBounds = await page.getByText('Auto-merge enabled', { exact: true })
    .locator('xpath=ancestor::div[1]').boundingBox()
  assert(footerBounds.x >= 0 && footerBounds.x + footerBounds.width <= 390)
  if (process.env.SCOPE_COMPONENT_SCREENSHOT) {
    await page.screenshot({ path: process.env.SCOPE_COMPONENT_SCREENSHOT, fullPage: true })
  }

  await page.getByRole('button', { name: 'Cancel auto-merge' }).click()
  const cancelDialog = page.getByRole('alertdialog')
  const dialogBounds = await cancelDialog.boundingBox()
  assert(dialogBounds.x >= 0 && dialogBounds.x + dialogBounds.width <= 390)
  await page.evaluate(() => window.refreshAutoMerge())
  await cancelDialog.getByText('aaaaaaaaaaaa → main', { exact: true }).waitFor()
  await cancelDialog.getByRole('button', { name: 'Cancel auto-merge' }).click()
  await page.getByRole('button', { name: 'Enable auto-merge' }).waitFor()
  assert.equal(await page.getByText(/Authorized by/).count(), 0)
  assert.deepEqual(await page.evaluate(() => window.calls[1]), {
    expected_intent_id: 'intent-1',
  })
  await page.evaluate(() => window.setAutoMergeIntentStatus('Stopped'))
  await page.getByRole('button', { name: 'Enable auto-merge' }).waitFor()
  assert.equal(await page.getByText(/Authorized by/).count(), 0)
  assert.deepEqual(errors, [])
})
