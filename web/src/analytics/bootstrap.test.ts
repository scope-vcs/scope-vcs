import assert from 'node:assert/strict'
import test from 'node:test'
import {
  analyticsClientOptions,
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

test('browser client keeps remote collection features disabled', () => {
  const options = analyticsClientOptions('https://scopevcs.com')

  assert.equal(options.api_host, '/e')
  assert.equal(options.ui_host, 'https://us.posthog.com')
  assert.equal(options.advanced_disable_flags, true)
  assert.equal(options.advanced_disable_feature_flags, true)
  assert.equal(options.advanced_disable_feature_flags_on_first_load, true)
  assert.equal(options.autocapture, false)
  assert.equal(options.capture_exceptions, false)
  assert.equal(options.capture_pageview, false)
  assert.equal(options.capture_performance, false)
  assert.equal(options.disable_session_recording, true)
  assert.equal(options.respect_dnt, true)
})
