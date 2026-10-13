import assert from 'node:assert/strict'
import test from 'node:test'
import { fixtureClerk, openFixture } from './support/fixture-server.mjs'

test('a reply resent with a different quote is a new attempt', async (t) => {
  const { base, errors, page } = await openFixture(t, 'request-reply', { aliases: [fixtureClerk], timeout: 30_000 })
  await page.goto(base, { waitUntil: 'domcontentloaded' })

  const composer = page.locator('form')
  const send = composer.getByRole('button', { name: 'Reply', exact: true })
  await page.getByRole('button', { name: 'Reply to alice' }).click()
  await composer.getByText('Cap retries at five.').waitFor()
  await composer.getByRole('textbox', { name: 'Reply' }).fill('Five matches the client default.')
  await send.click()
  await page.getByRole('alert').filter({ hasText: 'Service unavailable' }).waitFor()

  await page.getByRole('button', { name: 'Reply to bob' }).click()
  await composer.getByText('Cap retries at three.').waitFor()
  await send.click()
  await page.waitForFunction(() => window.calls.length === 2)
  await composer.waitFor({ state: 'detached' })

  const [failed, resent] = await page.evaluate(() => window.calls)
  assert.equal(failed.reply_to_reply_id, 'reply-alice')
  assert.equal(resent.reply_to_reply_id, 'reply-bob')
  assert.equal(resent.body_markdown, failed.body_markdown)
  assert.notEqual(resent.client_reply_id, failed.client_reply_id)
  assert.deepEqual(errors, [])
})
