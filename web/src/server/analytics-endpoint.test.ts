import assert from 'node:assert/strict'
import test from 'node:test'
import { analyticsEndpointResponse } from './analytics-endpoint-handler'

test('serves disabled runtime configuration without caching it', async () => {
  const response = await analyticsEndpointResponse(
    new Request('https://scopevcs.com/e/config'),
  )

  assert.ok(response)
  assert.equal(response.status, 200)
  assert.equal(response.headers.get('cache-control'), 'no-store')
  assert.equal(response.headers.get('content-type'), 'application/json')
  assert.equal(await response.text(), 'null')
})

test('rejects unlisted analytics proxy paths', async () => {
  const response = await analyticsEndpointResponse(
    new Request('https://scopevcs.com/e/flags/?v=2', { method: 'POST' }),
  )

  assert.ok(response)
  assert.equal(response.status, 404)
  assert.equal(response.headers.get('cache-control'), 'no-store')
})

test('does not contact PostHog when the runtime gate is inactive', async () => {
  let contacted = false
  const response = await analyticsEndpointResponse(new Request(
    'https://scopevcs.com/e/e/',
    { body: '{}', method: 'POST' },
  ), {
    fetchUpstream: async () => {
      contacted = true
      return new Response(null, { status: 204 })
    },
    runtime: {},
  })

  assert.ok(response)
  assert.equal(response.status, 404)
  assert.equal(contacted, false)
})
