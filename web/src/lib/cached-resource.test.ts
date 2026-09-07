import assert from 'node:assert/strict'
import test from 'node:test'
import { createCachedResource } from './cached-resource'

type Value = { text: string }
const value = (text: string): Value => ({ text })
const resource = () => createCachedResource<Value>({ maxEntries: 4 })

test('subscribers share one request which survives leaving and returning', async () => {
  const store = resource()
  const response = deferred<Value>()
  let requests = 0
  let firstUpdates = 0
  let secondUpdates = 0
  let signal!: AbortSignal
  const load = (nextSignal: AbortSignal) => {
    requests++
    signal = nextSignal
    return response.promise
  }
  const leaveFirst = store.subscribe('repo', () => firstUpdates++)
  const leaveSecond = store.subscribe('repo', () => secondUpdates++)
  const first = store.ensure('repo', '1', load)
  const second = store.ensure('repo', '1', load)
  assert.equal(first, second)
  await Promise.resolve()
  leaveFirst()
  leaveSecond()
  assert.equal(signal.aborted, false)
  const previousFirstUpdates = firstUpdates
  const previousSecondUpdates = secondUpdates
  response.resolve(value('retained'))
  await first
  assert.equal(firstUpdates, previousFirstUpdates)
  assert.equal(secondUpdates, previousSecondUpdates)
  assert.deepEqual(store.getSnapshot('repo').value, value('retained'))
  const leaveReturn = store.subscribe('repo', () => {})
  await store.ensure('repo', '1', load)
  assert.equal(requests, 1)
  leaveReturn()
})

test('returning while the previous request is pending also joins that request', async () => {
  const store = resource()
  const response = deferred<Value>()
  const leave = store.subscribe('repo', () => {})
  const first = store.ensure('repo', '1', () => response.promise)
  leave()
  const leaveReturn = store.subscribe('repo', () => {})
  const returning = store.ensure('repo', '1', async () => {
    assert.fail('a revisit must not start a second request')
  })
  assert.equal(returning, first)
  response.resolve(value('complete'))
  await returning
  assert.equal(store.getSnapshot('repo').pending, false)
  leaveReturn()
})

test('revision refresh preserves displayed value until the new value is ready', async () => {
  const store = resource()
  store.write('repo', value('old'), '1')
  const response = deferred<Value>()
  const refresh = store.ensure('repo', '2', () => response.promise)
  assert.deepEqual(store.getSnapshot('repo').value, value('old'))
  assert.equal(store.getSnapshot('repo').pending, true)
  response.resolve(value('new'))
  await refresh
  assert.deepEqual(store.getSnapshot('repo').value, value('new'))
  assert.equal(store.getSnapshot('repo').version, '2')
  assert.equal(store.getSnapshot('repo').pending, false)
})

test('invalidation marks the retained result stale and allows the same revision to reload', async () => {
  const store = resource()
  store.write('repo', value('before'), '1')
  let updates = 0
  store.subscribe('repo', () => updates++)
  store.invalidate('repo')
  assert.equal(updates, 1)
  assert.deepEqual(store.getSnapshot('repo').value, value('before'))
  assert.equal(store.getSnapshot('repo').stale, true)
  await store.ensure('repo', '1', async () => value('after'))
  assert.deepEqual(store.getSnapshot('repo').value, value('after'))
  assert.equal(store.getSnapshot('repo').stale, false)
})

test('a replaced revision aborts its old request and ignores its late success', async () => {
  const store = resource()
  const old = deferred<Value>()
  let oldSignal!: AbortSignal
  const first = store.ensure('repo', '1', (signal) => {
    oldSignal = signal
    return old.promise
  })
  await Promise.resolve()
  await store.ensure('repo', '2', async () => value('current'))
  assert.equal(oldSignal.aborted, true)
  old.resolve(value('obsolete'))
  await first
  assert.deepEqual(store.getSnapshot('repo').value, value('current'))
  assert.equal(store.getSnapshot('repo').version, '2')
})

test('invalidating an in-flight request suppresses its late failure', async () => {
  const store = resource()
  store.write('repo', value('retained'), '0')
  const old = deferred<Value>()
  const pending = store.ensure('repo', '1', () => old.promise)
  await Promise.resolve()
  store.invalidate('repo')
  old.reject(new Error('obsolete failure'))
  await pending
  assert.deepEqual(store.getSnapshot('repo').value, value('retained'))
  assert.equal(store.getSnapshot('repo').error, null)
  assert.equal(store.getSnapshot('repo').stale, true)
})

test('a refresh error retains the successful result and explicit retry recovers', async () => {
  const store = resource()
  store.write('repo', value('retained'), '1')
  const failure = new Error('offline')
  await store.ensure('repo', '2', async () => { throw failure })
  assert.deepEqual(store.getSnapshot('repo').value, value('retained'))
  assert.equal(store.getSnapshot('repo').error, failure)
  assert.equal(store.getSnapshot('repo').pending, false)
  await store.ensure('repo', '2', async () => {
    assert.fail('an error must not cause an automatic retry loop')
  })
  store.invalidate('repo')
  await store.ensure('repo', '2', async () => value('recovered'))
  assert.deepEqual(store.getSnapshot('repo').value, value('recovered'))
  assert.equal(store.getSnapshot('repo').error, null)
})

test('an initial failure is observable and retries without an old value', async () => {
  const store = resource()
  await store.ensure('repo', '1', async () => { throw new Error('unavailable') })
  assert.equal(store.getSnapshot('repo').value, null)
  assert.match(String(store.getSnapshot('repo').error), /unavailable/)
  store.invalidate('repo')
  await store.ensure('repo', '1', async () => value('ready'))
  assert.deepEqual(store.getSnapshot('repo').value, value('ready'))
})

test('identities keep values, requests, and notifications separate', async () => {
  const store = resource()
  let firstUpdates = 0
  let otherUpdates = 0
  store.subscribe('viewer-1:repo-1:member', () => firstUpdates++)
  store.subscribe('viewer-2:repo-1:guest', () => otherUpdates++)
  const first = store.ensure('viewer-1:repo-1:member', '1', async () => value('private'))
  const other = store.ensure('viewer-2:repo-1:guest', '1', async () => value('public'))
  assert.notEqual(first, other)
  await Promise.all([first, other])
  assert.deepEqual(store.read('viewer-1:repo-1:member'), value('private'))
  assert.deepEqual(store.read('viewer-2:repo-1:guest'), value('public'))
  assert.equal(firstUpdates, 2)
  assert.equal(otherUpdates, 2)
  store.invalidate('viewer-1:repo-1:member')
  assert.equal(firstUpdates, 3)
  assert.equal(otherUpdates, 2)
  assert.equal(store.getSnapshot('viewer-2:repo-1:guest').stale, false)
})

test('retention evicts least recently read resources by entry and weight limits', () => {
  const store = createCachedResource<Value>({
    maxEntries: 2, maxWeight: 6, weightOf: (item) => item.text.length,
  })
  store.write('a', value('aa'))
  store.write('b', value('bb'))
  store.read('a')
  store.write('c', value('cc'))
  assert.equal(store.peek('b'), null)
  assert.deepEqual(store.peek('a'), value('aa'))
  store.write('d', value('ddddd'))
  assert.equal(store.peek('a'), null)
  assert.equal(store.peek('c'), null)
  assert.deepEqual(store.stats(), { entries: 1, totalWeight: 5 })
})

test('server snapshots remain empty and stable after client data is loaded', async () => {
  const store = resource()
  const serverSnapshot = store.getServerSnapshot()
  await store.ensure('repo', '1', async () => value('private client data'))
  assert.deepEqual(store.getSnapshot('repo').value, value('private client data'))
  assert.equal(store.getServerSnapshot(), serverSnapshot)
  assert.equal(store.getServerSnapshot().value, null)
  assert.equal(store.getServerSnapshot().error, null)
})

test('writing authoritative data cancels and supersedes a pending load', async () => {
  const store = resource()
  const old = deferred<Value>()
  const pending = store.ensure('repo', '1', () => old.promise)
  await Promise.resolve()
  store.write('repo', value('mutation result'), '2')
  old.resolve(value('pre-mutation response'))
  await pending
  assert.deepEqual(store.getSnapshot('repo').value, value('mutation result'))
  assert.equal(store.getSnapshot('repo').version, '2')
})

test('clear notifies subscribers and prevents pending work repopulating cached data', async () => {
  const store = resource()
  const response = deferred<Value>()
  let signal!: AbortSignal
  let updates = 0
  store.subscribe('repo', () => updates++)
  const pending = store.ensure('repo', '1', (nextSignal) => {
    signal = nextSignal
    return response.promise
  })
  await Promise.resolve()
  store.clear()
  assert.equal(signal.aborted, true)
  assert.equal(updates, 2)
  response.resolve(value('discarded'))
  await pending
  assert.equal(store.peek('repo'), null)
  assert.equal(store.stats().entries, 0)
})

test('recovery invalidation marks all retained resources stale without dropping their data', () => {
  const store = resource()
  store.write('a', value('one'), '1')
  store.write('b', value('two'), '2')
  store.invalidateAll()
  assert.equal(store.getSnapshot('a').stale, true)
  assert.equal(store.getSnapshot('b').stale, true)
  assert.deepEqual(store.read('a'), value('one'))
  assert.deepEqual(store.read('b'), value('two'))
})

function deferred<T>() {
  let resolve!: (value: T) => void
  let reject!: (error: unknown) => void
  const promise = new Promise<T>((nextResolve, nextReject) => {
    resolve = nextResolve
    reject = nextReject
  })
  return { promise, reject, resolve }
}

test('recovery invalidation also cancels requests whose pending snapshot was evicted', async () => {
  const store = createCachedResource<Value>({ maxEntries: 1 })
  const first = deferred<Value>()
  const second = deferred<Value>()
  let firstSignal!: AbortSignal
  let secondSignal!: AbortSignal
  const firstRequest = store.ensure('a', '1', (signal) => {
    firstSignal = signal
    return first.promise
  })
  const secondRequest = store.ensure('b', '1', (signal) => {
    secondSignal = signal
    return second.promise
  })
  await Promise.resolve()
  store.invalidateAll()
  first.resolve(value('before recovery'))
  second.resolve(value('before recovery'))
  await Promise.all([firstRequest, secondRequest])
  assert.equal(firstSignal.aborted, true)
  assert.equal(secondSignal.aborted, true)
  assert.equal(store.peek('a'), null)
  assert.equal(store.peek('b'), null)
})

test('route loaders and mounted views share the same pending request', async () => {
  const store = resource()
  const response = deferred<Value>()
  let requests = 0
  const load = () => { requests++; return response.promise }
  const route = store.load('repo', '1', load)
  const view = store.ensure('repo', '1', load)
  response.resolve(value('shared'))
  assert.deepEqual(await route, value('shared'))
  await view
  assert.equal(requests, 1)
  assert.deepEqual(await store.load('repo', '1', load), value('shared'))
  assert.equal(requests, 1)
})

test('oversized results remain readable while mounted without exceeding retained cache budget', async () => {
  const store = createCachedResource<Value>({ maxEntries: 4, maxWeight: 3, weightOf: (item) => item.text.length })
  const leave = store.subscribe('large', () => {})
  assert.deepEqual(await store.load('large', '1', async () => value('oversized')), value('oversized'))
  assert.deepEqual(store.getSnapshot('large').value, value('oversized'))
  assert.equal(store.stats().entries, 0)
  await store.ensure('large', '1', async () => assert.fail('active data must remain usable'))
  leave()
  assert.equal(store.getSnapshot('large').value, null)
})

test('an uncached oversized route result still resolves for its caller', async () => {
  const store = createCachedResource<Value>({ maxEntries: 1, maxWeight: 1, weightOf: (item) => item.text.length })
  assert.deepEqual(await store.load('large', '', async () => value('oversized')), value('oversized'))
  assert.equal(store.stats().entries, 0)
})
