import assert from 'node:assert/strict'
import test from 'node:test'
import { openFixture } from '../support/fixture-server.mjs'

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
  await page.evaluate(() => window.selectStackWaiting())
  await page.getByRole('heading', { name: 'Handle quoted paths in child layer', exact: true }).waitFor()
  assert.equal(await page.getByRole('button', { name: 'Merge', exact: true }).count(), 0)
  await page.getByRole('button', { name: 'Enable auto-merge' }).click()
  const stackDialog = page.getByRole('alertdialog')
  await stackDialog.getByText('a'.repeat(40), { exact: true }).waitFor()
  await stackDialog.getByRole('button', { name: 'Enable auto-merge' }).click()
  await page.getByText('Auto-merge enabled', { exact: true }).waitFor()
  const waiting = page.getByText(/Authorized by you/)
  assert.equal(await waiting.textContent(), 'Authorized by you · aaaaaaaaaaaa · Waiting for the parent request to land')
  assert.deepEqual(await page.evaluate(() => window.calls[2]), {
    expected_head_oid: 'a'.repeat(40),
    expected_revision_id: 'revision-7d9f2f1b',
  })
  assert.equal(await page.evaluate(() => document.documentElement.scrollWidth > innerWidth), false)
  const waitingBounds = await waiting.boundingBox()
  assert(waitingBounds.x >= 0 && waitingBounds.x + waitingBounds.width <= 390)
  const lineHeight = await waiting.evaluate((element) => parseFloat(getComputedStyle(element).lineHeight))
  assert(waitingBounds.height > lineHeight, 'parent waiting text must wrap within the mobile footer')
  if (process.env.SCOPE_COMPONENT_SCREENSHOT) {
    const path = process.env.SCOPE_COMPONENT_SCREENSHOT.replace(/\.png$/, '')
    await page.screenshot({ path: `${path}-stack.png`, fullPage: true })
  }
  assert.deepEqual(errors, [])
})
