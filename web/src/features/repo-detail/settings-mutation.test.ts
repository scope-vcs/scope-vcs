import assert from 'node:assert/strict'
import test, { mock } from 'node:test'
import { mutateSettings } from './settings-mutation'

test('refresh failures are separate from the write result and write failures never refresh', async () => {
  let refreshErrors = 0
  assert.equal(await mutateSettings(Promise.resolve('saved'), async () => { throw new Error('offline') }, () => { refreshErrors += 1 }), 'saved')
  await new Promise((resolve) => setImmediate(resolve))
  assert.equal(refreshErrors, 1)
  const refresh = mock.fn(async () => {})
  const onRefreshError = mock.fn()
  await assert.rejects(mutateSettings(Promise.reject(new Error('write failed')), refresh, onRefreshError), /write failed/)
  await new Promise((resolve) => setImmediate(resolve))
  assert.equal(refresh.mock.callCount(), 0)
  assert.equal(onRefreshError.mock.callCount(), 0)
})
