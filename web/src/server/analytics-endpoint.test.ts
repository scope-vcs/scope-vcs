import assert from 'node:assert/strict'
import test from 'node:test'
import { analyticsEndpointResponse } from './analytics-endpoint-handler'

const runtime = {
  SCOPE_ANALYTICS_ENVIRONMENT: 'test',
  SCOPE_ANALYTICS_ORIGIN: 'https://test.scopevcs.com',
  POSTHOG_PROJECT_TOKEN: 'phc_fixture',
  SCOPE_ANALYTICS_GEOIP: 'enabled',
  RAILWAY_ENVIRONMENT_ID: 'railway-test',
}

test('GeoIP uses only the Railway client address after explicit activation', async (t) => {
  const fixtures: {
    name: string
    address?: string
    expected: string | null
    runtime?: Record<string, string | undefined>
    headers?: Record<string, string>
  }[] = [
    { name: 'IPv4', address: '8.8.8.8', expected: '8.8.8.8' },
    { name: 'IPv6', address: '2001:4860:4860::8888', expected: '2001:4860:4860::8888' },
    { name: 'no address', expected: null },
    { name: 'malformed address', address: 'not-an-ip', expected: null },
    { name: 'untrusted chain', address: '8.8.8.8, 1.1.1.1', expected: null },
    { name: 'disabled', address: '8.8.8.8', runtime: { SCOPE_ANALYTICS_GEOIP: undefined }, expected: null },
    { name: 'non-Railway host', address: '8.8.8.8', runtime: { RAILWAY_ENVIRONMENT_ID: undefined }, expected: null },
    { name: 'GPC', address: '8.8.8.8', headers: { 'sec-gpc': '1' }, expected: null },
    { name: 'DNT', address: '8.8.8.8', headers: { dnt: '1' }, expected: null },
  ]
  for (const fixture of fixtures) {
    await t.test(fixture.name, async () => {
      const deliveries: Request[] = []
      const response = await analyticsEndpointResponse(new Request(
        'https://test.scopevcs.com/e/e/?ip=1', {
          method: 'POST',
          headers: {
            'x-forwarded-for': '1.1.1.1, 203.0.113.1',
            ...(fixture.address ? { 'x-real-ip': fixture.address } : {}),
            ...fixture.headers,
          },
          body: JSON.stringify({
            event: '$pageview', uuid: 'event-id', ip: '1.1.1.1',
            properties: {
              distinct_id: 'scope_usr_fixture', route_name: 'home', token: 'phc_wrong',
              environment: 'production', source: 'backend', $ip: '1.1.1.1',
              $geoip_disable: false, $geoip_latitude: 12, $geoip_postal_code: '12345',
              $set: { $geoip_longitude: 34 },
            },
          }),
        },
      ), {
        runtime: { ...runtime, ...fixture.runtime },
        observeDelivery: () => {},
        fetchUpstream: async (input, init) => {
          deliveries.push(new Request(input, init))
          return Response.json({ status: 1 })
        },
      })
      assert.equal(response?.status, 200)
      assert.equal(deliveries.length, 1)
      const delivered = deliveries[0]
      const capture = await delivered.json()
      assert.equal(delivered.headers.get('x-forwarded-for'), fixture.expected)
      assert.equal(delivered.headers.get('x-real-ip'), null)
      assert.equal(capture.properties.$geoip_disable, !fixture.expected)
      assert.equal(capture.properties.$ip ?? null, fixture.expected)
      assert.equal(capture.properties.$geoip_latitude, undefined)
      assert.equal(capture.properties.$geoip_postal_code, undefined)
      assert.equal(capture.properties.$set, undefined)
      assert.equal(capture.ip, undefined)
      assert.equal(capture.properties.token, 'phc_fixture')
      assert.equal(capture.properties.environment, 'test')
      assert.equal(capture.properties.source, 'browser')
    })
  }
})

test('rejects unsupported capture formats before delivering anything', async () => {
  const deliveries: unknown[] = []
  for (const body of ['null', '[]', '{', '{"event":"$pageview"}', JSON.stringify({
    event: '$autocapture', uuid: 'id', properties: { distinct_id: 'visitor' },
  })]) {
    const response = await analyticsEndpointResponse(new Request(
      'https://test.scopevcs.com/e/e/', { body, method: 'POST' },
    ), {
      runtime, observeDelivery: () => {},
      fetchUpstream: async (input) => { deliveries.push(input); return Response.json({}) },
    })
    assert.equal(response?.status, 400)
  }
  const compressed = await analyticsEndpointResponse(new Request(
    'https://test.scopevcs.com/e/e/', {
      body: 'compressed', method: 'POST', headers: { 'content-encoding': 'gzip' },
    },
  ), {
    runtime, observeDelivery: () => {},
    fetchUpstream: async (input) => { deliveries.push(input); return Response.json({}) },
  })
  assert.equal(compressed?.status, 415)
  assert.equal(deliveries.length, 0)
})

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
