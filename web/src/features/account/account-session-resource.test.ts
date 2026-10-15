import assert from 'node:assert/strict'
import test from 'node:test'
import type { AccountSessionResponse } from '@/api/types.generated'
import { loadAccountSessionValue } from './account-session-resource'

const account = (id: string): AccountSessionResponse => ({
  identity: null,
  user: {
    email: `${id}@example.test`,
    email_verified: true,
    handle: id,
    id: `scope_usr_${id}`,
  },
})

test('transient failures retry within the configured bound', async () => {
  for (const [failures, result] of [[2, 'scope_usr_one'], [3, /still unavailable/]] as const) {
    let loads = 0
    const value = loadAccountSessionValue(
      async () => {
        loads += 1
        if (loads <= failures) throw new Error('still unavailable')
        return account('one')
      },
      new AbortController().signal,
      { retryDelays: [0, 0], wait: async () => {} },
    )
    if (typeof result === 'string') assert.equal((await value).account?.user?.id, result)
    else await assert.rejects(value, result)
    assert.equal(loads, 3)
  }
})

test('an aborted read stops retrying immediately', async () => {
  const controller = new AbortController()
  controller.abort()
  let loads = 0
  await assert.rejects(loadAccountSessionValue(
    async () => {
      loads += 1
      throw new Error('aborted')
    },
    controller.signal,
    { retryDelays: [0, 0], wait: async () => {} },
  ), /aborted/)
  assert.equal(loads, 1)
})
