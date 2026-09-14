import assert from 'node:assert/strict'
import test from 'node:test'
import { mutateSettings } from './settings-mutation'

test('refresh failures are separate from the write result and write failures never refresh', async () => {
  let refreshErrors = 0
  assert.equal(await mutateSettings(Promise.resolve('saved'), async () => { throw new Error('offline') }, () => { refreshErrors += 1 }), 'saved')
  await new Promise((resolve) => setImmediate(resolve))
  assert.equal(refreshErrors, 1)
  await assert.rejects(mutateSettings(Promise.reject(new Error('write failed')), async () => assert.fail('must not refresh'), () => assert.fail('write error')), /write failed/)
})
