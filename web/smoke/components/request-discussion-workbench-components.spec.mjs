import assert from 'node:assert/strict'
import test from 'node:test'
import { fixtureClerk, openFixture } from '../support/fixture-server.mjs'

test('a focused discussion evicted between its load and the view subscribing is read again whole', async (t) => {
  const { base, errors, page } = await openFixture(t, 'request-discussion-workbench', { aliases: [fixtureClerk], tailwind: true })
  await page.goto(base, { timeout: 30_000 })

  assert.equal(await page.getByLabel('Retained before the view subscribes').textContent(), 'false')
  const discussion = page.getByRole('region', { name: 'Request discussion' })
  await discussion.getByText('Which retry cap?').waitFor()
  assert.equal(await page.evaluate(() => window.fixture.server.pageLoads), 2)
  assert.equal(await discussion.getByText('Another open question').count(), 1, 'recovery must not keep only the focused discussion')
  assert.equal(await discussion.getByRole('button', { name: 'Load earlier discussions' }).isEnabled(), true)
  assert.deepEqual(errors, [])
})

test('leaving during an earlier-page load drops that load and leaves the next view ready to load', async (t) => {
  const { base, errors, page } = await openFixture(t, 'request-discussion-workbench', { aliases: [fixtureClerk], tailwind: true })
  await page.goto(base, { timeout: 30_000 })
  const discussion = page.getByRole('region', { name: 'Request discussion' })
  const loadEarlier = discussion.getByRole('button', { name: 'Load earlier discussions' })
  const toggle = page.getByRole('button', { name: 'Toggle discussion' })
  const olderLoads = () => page.evaluate(() => window.fixture.server.olderLoads)

  await loadEarlier.click()
  await page.waitForFunction(() => window.fixture.server.olderLoads === 1)
  assert.equal(await loadEarlier.getAttribute('aria-busy'), 'true')
  await toggle.click()
  await discussion.waitFor({ state: 'detached' })
  await toggle.click()
  await discussion.getByText('Which retry cap?').waitFor()
  assert.equal(await loadEarlier.getAttribute('aria-busy'), 'false')
  assert.equal(await loadEarlier.isEnabled(), true)

  await loadEarlier.click()
  await page.waitForFunction(() => window.fixture.server.olderLoads === 2)
  await page.evaluate(() => window.fixture.finishOlderLoad())
  await page.evaluate(() => new Promise((resolve) => requestAnimationFrame(() => requestAnimationFrame(resolve))))
  assert.equal(await loadEarlier.getAttribute('aria-busy'), 'true', 'the abandoned load settling must not end the one in flight')
  assert.equal(await discussion.getByText('An older question').count(), 0)

  await page.evaluate(() => window.fixture.finishOlderLoad())
  await discussion.getByText('An older question').waitFor()
  assert.equal(await loadEarlier.count(), 0)
  assert.equal(await olderLoads(), 2)
  assert.deepEqual(errors, [])
})
