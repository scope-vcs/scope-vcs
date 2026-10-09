import assert from 'node:assert/strict'
import { mock, test } from 'node:test'
import { markSessionReady, sessionReady, whenSessionReady } from './viewer-state'

const settle = () => new Promise<void>((resolve) => setImmediate(resolve))

test('waiting for the session ends at the bound when it never becomes ready', async (t) => {
  t.mock.timers.enable({ apis: ['setTimeout'] })
  const settled = mock.fn()
  void whenSessionReady(10_000).then(settled)
  t.mock.timers.tick(9_999)
  await settle()
  assert.equal(settled.mock.callCount(), 0)
  t.mock.timers.tick(1)
  await settle()
  assert.equal(settled.mock.callCount(), 1)
  assert.equal(sessionReady(), false)
})

test('waiting for the session ends when it becomes ready and never waits again afterwards', async () => {
  const settled = mock.fn()
  void whenSessionReady(10_000).then(settled)
  await settle()
  assert.equal(settled.mock.callCount(), 0)
  markSessionReady()
  await settle()
  assert.equal(settled.mock.callCount(), 1)
  assert.equal(sessionReady(), true)
  const immediate = mock.fn()
  void whenSessionReady(10_000).then(immediate)
  await settle()
  assert.equal(immediate.mock.callCount(), 1)
})
