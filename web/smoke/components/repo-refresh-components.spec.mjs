import assert from 'node:assert/strict'
import test from 'node:test'
import { fixtureClerk, openFixture } from '../support/fixture-server.mjs'

test('accepted summaries and reconnects reconcile the retained request queue without restarting the stream', async t => {
  const { base, errors, page } = await openFixture(t, 'repo-refresh', { aliases: [fixtureClerk], timeout: 5_000 })
  await page.goto(base, { timeout: 30_000 })
  await page.waitForFunction(() => window.fixture?.server.queueReads >= 4)
  await page.evaluate(async () => { window.fixture.submit(['new-request']); await window.fixture.refresh() })
  await page.locator('[data-count]').filter({ hasText: '1' }).waitFor()
  await page.getByRole('listitem').filter({ hasText: 'new-request' }).waitFor()
  assert.equal(await page.evaluate(() => window.fixture.server.connections), 1)

  const beforeNavigation = await page.evaluate(() => window.fixture.server.queueReads)
  await page.getByRole('button', { name: 'Navigate' }).click()
  await page.getByRole('button', { name: 'Navigate' }).click()
  await page.getByRole('listitem').filter({ hasText: 'new-request' }).waitFor()
  assert.equal(await page.evaluate(() => window.fixture.server.queueReads), beforeNavigation)

  await page.evaluate(async () => { window.fixture.submit(['replacement']); await window.fixture.refresh() })
  await page.getByRole('listitem').filter({ hasText: 'replacement' }).waitFor()
  assert.equal(await page.getByText('new-request', { exact: true }).count(), 0)
  assert.equal(await page.evaluate(() => window.fixture.server.connections), 1)

  const beforeLag = await page.evaluate(() => window.fixture.server.queueReads)
  await page.evaluate(() => { window.fixture.submit(['after-lag']); window.fixture.lag() })
  await page.getByRole('listitem').filter({ hasText: 'after-lag' }).waitFor()
  assert.equal(await page.evaluate(() => window.fixture.server.queueReads), beforeLag + 4)

  const beforeVersion = await page.evaluate(() => window.fixture.server.queueReads)
  await page.evaluate(async () => { window.fixture.advanceVersion(); window.fixture.submit(['new-version']); await window.fixture.refresh() })
  await page.getByRole('listitem').filter({ hasText: 'new-version' }).waitFor()
  assert.equal(await page.evaluate(() => window.fixture.server.queueReads), beforeVersion + 4)

  await page.evaluate(() => window.fixture.interrupt())
  await page.waitForFunction(() => window.fixture.server.streams.size === 0)
  await page.evaluate(() => window.fixture.submit(['during-disconnect']))
  await page.getByRole('listitem').filter({ hasText: 'during-disconnect' }).waitFor()
  assert.equal(await page.evaluate(() => window.fixture.server.connections), 2)
  assert.deepEqual(
    await page.evaluate(() => window.fixture.server.sinces.slice(1)),
    await page.evaluate(() => window.fixture.server.synced.slice(0, 1)),
    'a reconnect resumes from the last Synced watermark',
  )
  const settled = await page.evaluate(() => ({ reads: window.fixture.server.summaryReads, connections: window.fixture.server.connections }))
  await page.waitForTimeout(150)
  assert.deepEqual(await page.evaluate(() => ({ reads: window.fixture.server.summaryReads, connections: window.fixture.server.connections })), settled)
  assert.deepEqual(errors, [])
})

test('an open page reloads its repository data every ten minutes as a backstop', async t => {
  const { base, errors, page } = await openFixture(t, 'repo-refresh', { aliases: [fixtureClerk], timeout: 5_000 })
  await page.clock.install({ time: new Date('2026-11-17T12:00:00Z') })
  await page.goto(base, { timeout: 30_000 })
  await page.waitForFunction(() => window.fixture?.server.queueReads >= 4 && window.fixture.server.synced.length === 1)
  const queueReads = () => page.evaluate(() => window.fixture.server.queueReads)
  const before = await queueReads()
  await page.clock.runFor(9 * 60_000)
  assert.equal(await queueReads(), before)
  await page.clock.runFor(2 * 60_000)
  await page.waitForFunction((reads) => window.fixture.server.queueReads === reads + 4, before)
  assert.deepEqual(errors, [])
})

test('a failed summary refresh retries after two seconds until the layout unmounts', async t => {
  const { base, errors, page } = await openFixture(t, 'repo-refresh', { aliases: [fixtureClerk], timeout: 5_000 })
  await page.clock.install({ time: new Date('2026-11-17T12:00:00Z') })
  await page.goto(base, { timeout: 30_000 })
  await page.locator('[data-count]').filter({ hasText: '0' }).waitFor()
  await page.clock.pauseAt(new Date('2026-11-17T12:00:05Z'))
  const summaryReads = () => page.evaluate(() => window.fixture.server.summaryReads)

  const beforeFailure = await summaryReads()
  await page.evaluate(() => { window.fixture.server.summaryFailures = 1; window.fixture.submit(['recovered']); window.fixture.refresh() })
  await page.waitForFunction((reads) => window.fixture.server.summaryReads === reads + 1, beforeFailure)
  await page.clock.runFor(1_999)
  assert.equal(await summaryReads(), beforeFailure + 1)
  await page.clock.runFor(1)
  await page.locator('[data-count]').filter({ hasText: '1' }).waitFor()
  assert.equal(await summaryReads(), beforeFailure + 2)

  await page.evaluate(() => { window.fixture.server.summaryFailures = 1; window.fixture.refresh() })
  await page.waitForFunction((reads) => window.fixture.server.summaryReads === reads + 3, beforeFailure)
  await page.evaluate(() => window.fixture.unmount())
  await page.clock.runFor(10_000)
  assert.equal(await summaryReads(), beforeFailure + 3)
  assert.deepEqual(errors, [])
})
