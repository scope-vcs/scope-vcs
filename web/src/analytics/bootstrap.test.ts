import assert from 'node:assert/strict'
import test from 'node:test'
import { fetchAnalyticsRuntimeConfig } from './bootstrap'

test('runtime configuration uses the unauthenticated same-origin endpoint', async (t) => {
  const config = { environment: 'test', release: 'web-abc123', token: 'phc_test' }
  for (const body of [config, null]) {
    const fetch = t.mock.method(globalThis, 'fetch', async () => Response.json(body))
    const signal = new AbortController().signal

    assert.deepEqual(await fetchAnalyticsRuntimeConfig(signal), body)
    assert.equal(fetch.mock.callCount(), 1)
    assert.deepEqual(fetch.mock.calls[0]?.arguments, ['/e/config', {
      cache: 'no-store',
      credentials: 'omit',
      headers: { Accept: 'application/json' },
      referrerPolicy: 'no-referrer',
      signal,
    }])
    fetch.mock.restore()
  }
})
