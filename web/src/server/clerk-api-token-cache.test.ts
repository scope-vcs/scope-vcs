import * as assert from 'node:assert/strict'
import { test } from 'node:test'
import { createClerkApiTokenCache, type ClerkApiTokenOwner } from './clerk-api-token-cache'

const alice: ClerkApiTokenOwner = { sessionId: 'sess_alice', template: 'scope_api', userId: 'user_alice' }
const bob: ClerkApiTokenOwner = { sessionId: 'sess_bob', template: 'scope_api', userId: 'user_bob' }

test('reuses a session token until 30 seconds before it expires', async () => {
  let nowMs = 1_000_000
  const cache = createClerkApiTokenCache({ now: () => nowMs })
  let mints = 0
  const mint = async () => jwt(`token-${++mints}`, nowMs + 300_000)

  const first = await cache.read(alice, mint)
  nowMs += 269_000
  assert.equal(await cache.read(alice, mint), first)
  nowMs += 1_000
  assert.notEqual(await cache.read(alice, mint), first)
  assert.equal(mints, 2)
})

test('concurrent users never receive each other’s token', async () => {
  const cache = createClerkApiTokenCache({ now: () => 0 })
  const owners = [
    alice,
    bob,
    { ...alice, sessionId: 'sess_alice_other_device' },
    { ...alice, userId: 'user_mallory' },
    { ...bob, template: 'other_template' },
  ]
  const releases: Array<() => void> = []
  const mintFor = (owner: ClerkApiTokenOwner) => () =>
    new Promise<string>((resolve) => {
      releases.push(() => resolve(jwt(JSON.stringify(owner))))
    })

  const reads = owners.flatMap((owner) => [0, 1, 2].map(() => cache.read(owner, mintFor(owner))))
  assert.equal(releases.length, owners.length, 'one Clerk mint per owner')
  releases.reverse().forEach((release) => release())

  const tokens = await Promise.all(reads)
  owners.forEach((owner, index) => {
    for (const token of tokens.slice(index * 3, index * 3 + 3)) {
      assert.deepEqual(subject(token), owner)
    }
  })
  for (const owner of owners) {
    assert.deepEqual(subject(await cache.read(owner, unexpectedMint)), owner)
  }
})

test('does not keep failed, missing, or undated tokens', async () => {
  const cache = createClerkApiTokenCache({ now: () => 0 })

  await assert.rejects(cache.read(alice, async () => { throw new Error('clerk down') }))
  assert.equal(await cache.read(alice, async () => null), null)
  assert.equal(await cache.read(alice, async () => 'not-a-jwt'), 'not-a-jwt')
  assert.equal(await cache.read(alice, async () => jwt('fresh')), jwt('fresh'))
})

test('evicts the least recently used session when full', async () => {
  const cache = createClerkApiTokenCache({ maxEntries: 1, now: () => 0 })

  await cache.read(alice, async () => jwt('alice'))
  await cache.read(bob, async () => jwt('bob'))
  assert.equal(await cache.read(bob, unexpectedMint), jwt('bob'))
  assert.equal(await cache.read(alice, async () => jwt('alice-again')), jwt('alice-again'))
})

function jwt(sub: string, expiresAtMs = 300_000) {
  const encode = (value: object) => Buffer.from(JSON.stringify(value)).toString('base64url')
  return `${encode({ alg: 'RS256' })}.${encode({ exp: expiresAtMs / 1000, sub })}.sig`
}

function subject(token: string | null) {
  assert.ok(token)
  return JSON.parse(JSON.parse(Buffer.from(token.split('.')[1]!, 'base64url').toString()).sub)
}

async function unexpectedMint(): Promise<string> {
  throw new Error('expected a cached token')
}
