import assert from 'node:assert/strict'
import test from 'node:test'
import { AnalyticsClient } from './client'
import { analyticsEventContext, applyAnalyticsIdentityTransition, registerAnalyticsEventContext } from './client-identity'
import { pageViewProperties } from './privacy'

test('real client preserves event identities and clears persisted user on logout', async () => {
  const storage = new Map<string, string>()
  const captured: Array<{ event: string; properties: Record<string, unknown> }> = []
  const restore = browserGlobals(storage, '0', async (_input, init) => {
    captured.push(JSON.parse(init?.body as string))
    return Response.json({ status: 1 })
  })
  try {
    const context = analyticsEventContext({ environment: 'test', release: null, token: 'phc_test' })
    const client = new AnalyticsClient('phc_test', 'https://scopevcs.com')
    registerAnalyticsEventContext(client, context)
    const anonymousId = client.get_distinct_id()
    client.capture('$pageview', pageViewProperties(
      { name: 'home', path: '/' },
      { origin: 'https://scopevcs.com', referrer: '', search: '' },
    ))
    await until(() => captured.length === 1)
    applyAnalyticsIdentityTransition(client, 'scope_usr_one', context)
    await until(() => captured.length === 2)
    applyAnalyticsIdentityTransition(client, null, context)
    const afterLogout = client.get_distinct_id()
    assert.notEqual(afterLogout, anonymousId)
    assert.equal(storage.has('scope_analytics_user_id'), false)
    client.capture('$pageview', pageViewProperties(
      { name: 'home', path: '/' },
      { origin: 'https://scopevcs.com', referrer: '', search: '' },
    ))
    await until(() => captured.length === 3)
    assert.deepEqual(captured.map(value => value.event), ['$pageview', '$identify', '$pageview'])
    assert.equal(captured[0].properties.distinct_id, anonymousId)
    assert.equal(captured[1].properties.distinct_id, 'scope_usr_one')
    assert.equal(captured[1].properties.$anon_distinct_id, anonymousId)
    assert.equal(captured[1].properties.$process_person_profile, true)
    assert.equal(captured[2].properties.distinct_id, afterLogout)
    assert.equal(captured[2].properties.$process_person_profile, false)
  } finally {
    restore()
  }
})

test('DNT prevents capture and identity storage', () => {
  const storage = new Map<string, string>()
  let requests = 0
  const restore = browserGlobals(storage, '1', async () => {
    requests++
    return Response.json({ status: 1 })
  })
  try {
    const client = new AnalyticsClient('phc_test', 'https://scopevcs.com')
    client.identify('scope_usr_one')
    client.capture('$pageview', { route_name: 'home' })
    assert.equal(requests, 0)
    assert.equal(storage.size, 0)
  } finally {
    restore()
  }
})

function browserGlobals(
  storage: Map<string, string>,
  dnt: string,
  fetcher: typeof fetch,
) {
  const globals = ['window', 'navigator', 'localStorage', 'fetch'] as const
  const previous = globals.map(name => Object.getOwnPropertyDescriptor(globalThis, name))
  const window = new EventTarget()
  Object.defineProperty(globalThis, 'window', { configurable: true, value: window })
  Object.defineProperty(globalThis, 'navigator', { configurable: true, value: { doNotTrack: dnt } })
  Object.defineProperty(globalThis, 'localStorage', { configurable: true, value: {
    getItem: (key: string) => storage.get(key) ?? null,
    setItem: (key: string, value: string) => storage.set(key, value),
    removeItem: (key: string) => storage.delete(key),
  } })
  Object.defineProperty(globalThis, 'fetch', { configurable: true, value: fetcher })
  return () => globals.forEach((name, index) => {
    const descriptor = previous[index]
    if (descriptor) Object.defineProperty(globalThis, name, descriptor)
    else Reflect.deleteProperty(globalThis, name)
  })
}

async function until(condition: () => boolean) {
  const deadline = Date.now() + 2_000
  while (!condition()) {
    assert.ok(Date.now() < deadline)
    await new Promise(resolve => setTimeout(resolve, 10))
  }
}
