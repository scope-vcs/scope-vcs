import assert from 'node:assert/strict'
import test from 'node:test'
import type { AccountSessionResponse } from '@/api/types.generated'
import {
  accountSessionIdentity,
  accountSessionResource,
  activateAccountSessionViewer,
  loadAccountSessionValue,
  resetAccountSessionResource,
  type AccountSessionLoader,
} from './account-session-resource'

const account = (id: string): AccountSessionResponse => ({
  identity: null,
  user: {
    email: `${id}@example.test`,
    email_verified: true,
    handle: id,
    id: `scope_usr_${id}`,
  },
})

const readSession = async (viewerId: string, load: AccountSessionLoader) => {
  const value = await accountSessionResource.load(
    accountSessionIdentity(viewerId),
    '',
    (signal) => loadAccountSessionValue(load, signal),
  )
  return value.account
}

test.beforeEach(resetAccountSessionResource)

test('navigation reuses the retained account session for the same viewer', async () => {
  let loads = 0
  const load = async () => {
    loads += 1
    return account('one')
  }

  assert.equal((await readSession('clerk_one', load))?.user?.id, 'scope_usr_one')
  assert.equal((await readSession('clerk_one', load))?.user?.id, 'scope_usr_one')
  assert.equal(loads, 1)
})

test('a transient failure is retried within a bounded attempt count', async () => {
  let loads = 0
  const value = await loadAccountSessionValue(
    async () => {
      loads += 1
      if (loads < 3) throw new Error('temporary')
      return account('one')
    },
    new AbortController().signal,
    { retryDelays: [0, 0], wait: async () => {} },
  )

  assert.equal(value.account?.user?.id, 'scope_usr_one')
  assert.equal(loads, 3)
})

test('viewer changes discard retained data and reject a late previous-viewer write', async () => {
  let resolveFirst: ((value: AccountSessionResponse) => void) | undefined
  // The session boundary owns viewer activation; subscribers only read.
  activateAccountSessionViewer('clerk_one')
  const first = readSession('clerk_one', () => (
    new Promise<AccountSessionResponse>((resolve) => {
      resolveFirst = resolve
    })
  ))
  await Promise.resolve()

  activateAccountSessionViewer('anonymous')
  await readSession('clerk_two', async () => account('two'))
  resolveFirst?.(account('one'))
  await assert.rejects(first, /no longer available/)

  let loads = 0
  const reloaded = await readSession('clerk_one', async () => {
    loads += 1
    return account('one')
  })

  assert.equal(reloaded?.user?.id, 'scope_usr_one')
  assert.equal(loads, 1)
})

test('retry attempts stop at the configured bound', async () => {
  let loads = 0
  await assert.rejects(loadAccountSessionValue(
    async () => {
      loads += 1
      throw new Error('still unavailable')
    },
    new AbortController().signal,
    { retryDelays: [0, 0], wait: async () => {} },
  ), /still unavailable/)
  assert.equal(loads, 3)
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
