import assert from 'node:assert/strict'
import test from 'node:test'
import { fixtureClerk, openFixture } from '../support/fixture-server.mjs'

test('a discussion evicted between its load and the view subscribing is read again and renders', async (t) => {
  const { base, errors, page } = await openFixture(t, 'request-discussion-workbench', { aliases: [fixtureClerk], tailwind: true })
  await page.goto(base, { timeout: 30_000 })

  assert.equal(await page.getByLabel('Retained before the view subscribes').textContent(), 'false')
  const discussion = page.getByRole('region', { name: 'Request discussion' })
  await discussion.getByText('Which retry cap?').waitFor()
  assert.equal(await page.evaluate(() => window.fixture.server.pageLoads), 2)
  assert.deepEqual(errors, [])
})
