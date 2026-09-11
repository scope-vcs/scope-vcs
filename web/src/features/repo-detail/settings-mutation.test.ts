import assert from 'node:assert/strict'
import test from 'node:test'
import { mutateSettings } from './settings-mutation'

test('an invitation URL is delivered even while route refresh keeps retrying', async () => {
  const invite = { invite_url: 'https://scope.test/invites/token' }
  let refreshing = false
  const saved = await mutateSettings(Promise.resolve(invite), () => {
    refreshing = true
    return new Promise(() => {})
  }, () => assert.fail('refresh has not failed'))
  assert.equal(saved, invite)
  assert.equal(refreshing, true)
})

test('refresh failures are separate from the write result and write failures never refresh', async () => {
  let refreshErrors = 0
  assert.equal(await mutateSettings(Promise.resolve('saved'), async () => { throw new Error('offline') }, () => { refreshErrors += 1 }), 'saved')
  await new Promise((resolve) => setImmediate(resolve))
  assert.equal(refreshErrors, 1)
  await assert.rejects(mutateSettings(Promise.reject(new Error('write failed')), async () => assert.fail('must not refresh'), () => assert.fail('write error')), /write failed/)
})
