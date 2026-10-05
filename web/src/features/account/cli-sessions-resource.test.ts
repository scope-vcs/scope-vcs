import assert from 'node:assert/strict'
import test from 'node:test'
import { resetViewerState } from '../../lib/viewer-state'
import {
  acceptCliSessionsHandoff,
  loadCliSessionsHandoff,
  readRetainedCliSessions,
  retainRevokedCliSession,
} from './cli-sessions-resource'

const session = (id: string) => ({
  id,
  label: id,
  created_at_unix: 1,
  last_used_at_unix: null,
  expires_at_unix: 2,
})

test('a loader started before revocation cannot restore the revoked session', async () => {
  let release: (() => void) | undefined
  const held = new Promise<void>((resolve) => { release = resolve })
  const current = await loadCliSessionsHandoff(async () => ({
    viewerId: 'account-revocation',
    sessions: { sessions: [session('first'), session('second')] },
  }))
  assert.deepEqual(acceptCliSessionsHandoff('account-revocation', current)?.sessions.map(({ id }) => id), ['first', 'second'])
  const stale = loadCliSessionsHandoff(async () => {
    await held
    return { viewerId: 'account-revocation', sessions: { sessions: [session('first'), session('second')] } }
  })
  retainRevokedCliSession('account-revocation', 'second')
  assert.deepEqual(readRetainedCliSessions('account-revocation')?.sessions.map(({ id }) => id), ['first'])
  release?.()
  assert.equal(acceptCliSessionsHandoff('account-revocation', await stale), null)
  assert.deepEqual(readRetainedCliSessions('account-revocation')?.sessions.map(({ id }) => id), ['first'])
})

test('current handoffs publish only for the matching viewer', async () => {
  const handoff = await loadCliSessionsHandoff(async () => ({
    viewerId: 'account-viewer-a',
    sessions: { sessions: [session('owned')] },
  }))
  assert.equal(acceptCliSessionsHandoff('account-viewer-b', handoff), null)
  assert.equal(readRetainedCliSessions('account-viewer-b'), null)
  assert.equal(acceptCliSessionsHandoff(null, handoff), null)
  assert.deepEqual(acceptCliSessionsHandoff('account-viewer-a', handoff)?.sessions.map(({ id }) => id), ['owned'])
  assert.deepEqual(readRetainedCliSessions('account-viewer-a')?.sessions.map(({ id }) => id), ['owned'])
})

test('a prior viewer session cannot repopulate the cache after viewer reset', async () => {
  const handoff = await loadCliSessionsHandoff(async () => ({
    viewerId: 'account-viewer-returned',
    sessions: { sessions: [session('old-session')] },
  }))
  assert.notEqual(acceptCliSessionsHandoff('account-viewer-returned', handoff), null)
  resetViewerState()
  assert.equal(readRetainedCliSessions('account-viewer-returned'), null)
  assert.equal(acceptCliSessionsHandoff('account-viewer-returned', handoff), null)
  assert.equal(readRetainedCliSessions('account-viewer-returned'), null)
  const current = await loadCliSessionsHandoff(async () => ({
    viewerId: 'account-viewer-returned',
    sessions: { sessions: [session('new-session')] },
  }))
  assert.deepEqual(acceptCliSessionsHandoff('account-viewer-returned', current)?.sessions.map(({ id }) => id), ['new-session'])
})
