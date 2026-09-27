import assert from 'node:assert/strict'
import test from 'node:test'
import { AnalyticsClient } from './client'
import { analyticsEventContext, applyAnalyticsIdentityTransition, registerAnalyticsEventContext } from './client-identity'
import { pageViewProperties } from './privacy'

const homePageView = () => pageViewProperties(
  { name: 'home', path: '/' },
  { origin: 'https://scopevcs.com', referrer: '', search: '' },
)

test('real client preserves event identities and stores nothing on the device', async () => {
  const storage = new Map<string, string>()
  const captured: Array<{ event: string; properties: Record<string, unknown> }> = []
  const restore = browserGlobals(storage, { doNotTrack: '0' }, async (_input, init) => {
    captured.push(JSON.parse(init?.body as string))
    return Response.json({ status: 1 })
  })
  try {
    const context = analyticsEventContext({ environment: 'test', release: null, token: 'phc_test' })
    const client = new AnalyticsClient('phc_test', 'https://scopevcs.com')
    registerAnalyticsEventContext(client, context)
    const anonymousId = client.get_distinct_id()
    client.capture('$pageview', homePageView())
    await until(() => captured.length === 1)
    applyAnalyticsIdentityTransition(client, 'scope_usr_one', context)
    await until(() => captured.length === 2)
    applyAnalyticsIdentityTransition(client, null, context)
    const afterLogout = client.get_distinct_id()
    assert.notEqual(afterLogout, anonymousId)
    client.capture('$pageview', homePageView())
    await until(() => captured.length === 3)
    assert.deepEqual(captured.map(value => value.event), ['$pageview', '$identify', '$pageview'])
    assert.equal(captured[0].properties.distinct_id, anonymousId)
    assert.equal(captured[1].properties.distinct_id, 'scope_usr_one')
    assert.equal(captured[1].properties.$anon_distinct_id, anonymousId)
    assert.equal(captured[1].properties.$process_person_profile, true)
    assert.equal(captured[2].properties.distinct_id, afterLogout)
    assert.equal(captured[2].properties.$process_person_profile, false)
    assert.equal(storage.size, 0)
  } finally {
    restore()
  }
})

test('identify without anonymous events switches identity without $identify', async () => {
  const captured: Array<{ event: string; properties: Record<string, unknown> }> = []
  const restore = browserGlobals(new Map(), { doNotTrack: '0' }, async (_input, init) => {
    captured.push(JSON.parse(init?.body as string))
    return Response.json({ status: 1 })
  })
  try {
    const context = analyticsEventContext({ environment: 'test', release: null, token: 'phc_test' })
    const client = new AnalyticsClient('phc_test', 'https://scopevcs.com')
    registerAnalyticsEventContext(client, context)
    client.capture('unexpected_event')
    applyAnalyticsIdentityTransition(client, 'scope_usr_one', context)
    client.capture('$pageview', homePageView())
    // Replacing a signed-in user resets first, leaving no anonymous events to merge.
    applyAnalyticsIdentityTransition(client, 'scope_usr_two', context)
    client.capture('$pageview', homePageView())
    await until(() => captured.length === 2)
    assert.deepEqual(captured.map(value => value.event), ['$pageview', '$pageview'])
    assert.deepEqual(captured.map(value => value.properties.distinct_id), ['scope_usr_one', 'scope_usr_two'])
  } finally {
    restore()
  }
})

for (const [name, preference] of [
  ['DNT', { doNotTrack: '1' }],
  ['GPC', { doNotTrack: '0', globalPrivacyControl: true }],
] as const) {
  test(`${name} prevents capture and identity storage`, () => {
    const storage = new Map<string, string>()
    let requests = 0
    const restore = browserGlobals(storage, preference, async () => {
      requests++
      return Response.json({ status: 1 })
    })
    try {
      const client = new AnalyticsClient('phc_test', 'https://scopevcs.com')
      client.capture('$pageview', homePageView())
      client.identify('scope_usr_one')
      client.capture('$pageview', homePageView())
      assert.equal(requests, 0)
      assert.equal(storage.size, 0)
    } finally {
      restore()
    }
  })
}

function browserGlobals(
  storage: Map<string, string>,
  navigator: { doNotTrack: string; globalPrivacyControl?: boolean },
  fetcher: typeof fetch,
) {
  const globals = ['window', 'navigator', 'localStorage', 'fetch'] as const
  const previous = globals.map(name => Object.getOwnPropertyDescriptor(globalThis, name))
  const window = new EventTarget()
  Object.defineProperty(globalThis, 'window', { configurable: true, value: window })
  Object.defineProperty(globalThis, 'navigator', { configurable: true, value: navigator })
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
