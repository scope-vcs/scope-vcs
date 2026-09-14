import assert from 'node:assert/strict'
import test from 'node:test'
import type { AccountSessionResponse } from '@/api/types.generated'
import {
  activateAccountSessionViewer,
  loadAccountSessionForViewer,
  resetAccountSessionResource,
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

test.beforeEach(resetAccountSessionResource)

test('navigation reuses the retained account session for the same viewer', async () => {
  let loads = 0
  const load = async () => {
    loads += 1
    return account('one')
  }

  assert.equal((await loadAccountSessionForViewer('clerk_one', load))?.user?.id, 'scope_usr_one')
  assert.equal((await loadAccountSessionForViewer('clerk_one', load))?.user?.id, 'scope_usr_one')
  assert.equal(loads, 1)
})

test('a transient failure is retried within a bounded attempt count', async () => {
  let loads = 0
  const result = await loadAccountSessionForViewer(
    'clerk_one',
    async () => {
      loads += 1
      if (loads < 3) throw new Error('temporary')
      return account('one')
    },
    { retryDelays: [0, 0], wait: async () => {} },
  )

  assert.equal(result?.user?.id, 'scope_usr_one')
  assert.equal(loads, 3)
})

test('viewer changes discard retained data and reject a late previous-viewer write', async () => {
  let resolveFirst: ((value: AccountSessionResponse) => void) | undefined
  const first = loadAccountSessionForViewer('clerk_one', () => (
    new Promise<AccountSessionResponse>((resolve) => {
      resolveFirst = resolve
    })
  ))
  await Promise.resolve()

  activateAccountSessionViewer('anonymous')
  await loadAccountSessionForViewer('clerk_two', async () => account('two'))
  resolveFirst?.(account('one'))
  await assert.rejects(first, /no longer available/)

  let loads = 0
  const reloaded = await loadAccountSessionForViewer('clerk_one', async () => {
    loads += 1
    return account('one')
  })

  assert.equal(reloaded?.user?.id, 'scope_usr_one')
  assert.equal(loads, 1)
})

test('retry attempts stop at the configured bound', async () => {
  let loads = 0
  await assert.rejects(loadAccountSessionForViewer(
    'clerk_one',
    async () => {
      loads += 1
      throw new Error('still unavailable')
    },
    { retryDelays: [0, 0], wait: async () => {} },
  ), /still unavailable/)
  assert.equal(loads, 3)
})
