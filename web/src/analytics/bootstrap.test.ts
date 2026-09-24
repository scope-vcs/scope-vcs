import assert from 'node:assert/strict'
import test from 'node:test'
import {
  fetchAnalyticsRuntimeConfig,
} from './bootstrap'

test('runtime configuration uses the unauthenticated same-origin endpoint', async () => {
  let request: { input: RequestInfo | URL; init?: RequestInit } | null = null
  const fetcher: typeof fetch = async (input, init) => {
    request = { input, init }
    return Response.json({
      environment: 'test',
      release: 'web-abc123',
      token: 'phc_test',
    })
  }
  const signal = new AbortController().signal

  assert.deepEqual(await fetchAnalyticsRuntimeConfig(signal, fetcher), {
    environment: 'test',
    release: 'web-abc123',
    token: 'phc_test',
  })
  assert.deepEqual(request, {
    input: '/e/config',
    init: {
      cache: 'no-store',
      credentials: 'omit',
      headers: { Accept: 'application/json' },
      referrerPolicy: 'no-referrer',
      signal,
    },
  })
})

test('disabled runtime configuration does not initialize analytics', async () => {
  const fetcher: typeof fetch = async () => Response.json(null)
  assert.equal(
    await fetchAnalyticsRuntimeConfig(new AbortController().signal, fetcher),
    null,
  )
})
