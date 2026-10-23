import assert from 'node:assert/strict'
import { createServer, type RequestListener } from 'node:http'
import { gzipSync } from 'node:zlib'
import test from 'node:test'
import { proxyAnalyticsCapture } from './analytics-proxy'

const ignoreDelivery = () => {}
const config = { environment: 'test' as const, token: 'phc_fixture', release: null }
const payload = JSON.stringify({
  event: '$pageview', uuid: 'event-id',
  properties: { distinct_id: 'visitor', route_name: 'home' },
})

test('forwards a sanitized browser event without application headers or query overrides', async () => {
  let received: Request | undefined
  const observations: unknown[] = []

  const response = await proxyAnalyticsCapture(new Request(
    'https://test.scopevcs.com/e/e/?ip=1&_=123',
    {
      body: payload,
      headers: {
        authorization: 'Bearer application-secret',
        'content-type': 'application/json',
        cookie: '__session=application-secret',
        referer: 'https://scopevcs.com/private?token=secret',
        'x-forwarded-for': '203.0.113.10',
      },
      method: 'POST',
    },
  ), config, {
    fetchUpstream: async (input, init) => {
      received = new Request(input, init)
      return new Response('{"status":1}', {
        headers: { 'content-type': 'application/json', 'x-internal': 'secret' },
        status: 202,
      })
    },
    observeDelivery: (observation) => observations.push(observation),
  })

  assert.equal(response.status, 202)
  assert.equal(response.headers.get('cache-control'), 'no-store')
  assert.equal(response.headers.get('x-internal'), null)
  assert.ok(received)
  assert.equal(received.url, 'https://us.i.posthog.com/e/')
  assert.equal(received.headers.get('authorization'), null)
  assert.equal(received.headers.get('cookie'), null)
  assert.equal(received.headers.get('referer'), null)
  assert.equal(received.headers.get('x-forwarded-for'), null)
  assert.equal(received.headers.get('content-encoding'), null)
  assert.deepEqual(
    await received.json(),
    {
      event: '$pageview', uuid: 'event-id', properties: {
        distinct_id: 'visitor', route_name: 'home', $current_url: 'https://test.scopevcs.com/',
        $host: 'test.scopevcs.com', $pathname: '/', $geoip_disable: true,
        token: 'phc_fixture', environment: 'test', release: null, source: 'browser',
      },
    },
  )
  assert.equal(observations.length, 1)
  const observation = observations[0] as { durationMs: number; status: number }
  assert.equal(observation.status, 202)
  assert.ok(observation.durationMs >= 0)
})

test('decodes a real compressed upstream response without stale transport headers', async () => {
  const compressed = gzipSync('{"status":1}')
  await withStubServer((_request, response) => {
    response.writeHead(202, {
      'content-encoding': 'gzip',
      'content-length': compressed.byteLength,
      'content-type': 'application/json',
    })
    response.end(compressed)
  }, async (stubUrl) => {
    const response = await proxyAnalyticsCapture(new Request(
      'https://test.scopevcs.com/e/e/',
      { body: payload, method: 'POST' },
    ), config, {
      fetchUpstream: (input, init) => {
        assert.equal(input.toString(), 'https://us.i.posthog.com/e/')
        return fetch(stubUrl, init)
      },
      observeDelivery: ignoreDelivery,
    })

    assert.equal(response.status, 202)
    assert.equal(response.headers.get('content-encoding'), null)
    assert.equal(response.headers.get('content-length'), null)
    assert.equal(response.headers.get('content-type'), 'application/json')
    assert.equal(await response.text(), '{"status":1}')
  })
})

test('returns the upstream status without retrying', async () => {
  let attempts = 0
  const response = await proxyAnalyticsCapture(new Request(
    'https://test.scopevcs.com/e/e/',
    { body: payload, method: 'POST' },
  ), config, {
    fetchUpstream: async () => {
      attempts += 1
      return new Response('rate limited', { status: 429 })
    },
    observeDelivery: ignoreDelivery,
  })

  assert.equal(attempts, 1)
  assert.equal(response.status, 429)
  assert.equal(await response.text(), 'rate limited')
})

test('maps an upstream transport failure without exposing its error', async () => {
  const response = await proxyAnalyticsCapture(new Request(
    'https://test.scopevcs.com/e/e/',
    { body: payload, method: 'POST' },
  ), config, {
    fetchUpstream: async () => {
      throw new Error('stub secret')
    },
    observeDelivery: ignoreDelivery,
  })

  assert.equal(response.status, 502)
  assert.equal(await response.text(), 'Analytics upstream unavailable.')
})

test('rejects an oversized capture before contacting the upstream', async () => {
  let contacted = false
  const response = await proxyAnalyticsCapture(new Request(
    'https://test.scopevcs.com/e/e/',
    {
      body: new Uint8Array(1024 * 1024 + 1),
      method: 'POST',
    },
  ), config, {
    fetchUpstream: async () => {
      contacted = true
      return new Response(null, { status: 204 })
    },
    observeDelivery: ignoreDelivery,
  })

  assert.equal(response.status, 413)
  assert.equal(contacted, false)
})

test('times out and cancels a stalled incoming body before contacting the upstream', async () => {
  let bodyCanceled = false
  let contacted = false
  const stalledBody = new ReadableStream<Uint8Array>({
    cancel() {
      bodyCanceled = true
    },
  })

  const response = await proxyAnalyticsCapture(new Request(
    'https://test.scopevcs.com/e/e/',
    {
      body: stalledBody,
      method: 'POST',
      duplex: 'half',
    } as RequestInit & { duplex: 'half' },
  ), config, {
    fetchUpstream: async () => {
      contacted = true
      return new Response(null, { status: 204 })
    },
    observeDelivery: ignoreDelivery,
    timeoutMs: 20,
  })

  assert.equal(response.status, 408)
  assert.equal(await response.text(), 'Analytics request timed out.')
  assert.equal(contacted, false)
  assert.equal(bodyCanceled, true)
})

test('keeps the timeout active while reading the upstream response body', async () => {
  await withStubServer((_request, response) => {
    response.writeHead(200, { 'content-type': 'application/json' })
    response.write('{')
  }, async (stubUrl) => {
    const response = await proxyAnalyticsCapture(new Request(
      'https://test.scopevcs.com/e/e/',
      { body: payload, method: 'POST' },
    ), config, {
      fetchUpstream: (_input, init) => fetch(stubUrl, init),
      observeDelivery: ignoreDelivery,
      timeoutMs: 20,
    })

    assert.equal(response.status, 504)
    assert.equal(await response.text(), 'Analytics upstream timed out.')
  })
})

async function withStubServer(
  handler: RequestListener,
  run: (url: string) => Promise<void>,
) {
  const server = createServer(handler)
  await new Promise<void>((resolve) => server.listen(0, '127.0.0.1', resolve))
  const address = server.address()
  assert.ok(address && typeof address === 'object')

  try {
    await run(`http://127.0.0.1:${address.port}/capture`)
  } finally {
    server.closeAllConnections()
    await new Promise<void>((resolve, reject) => server.close((error) => {
      if (error) reject(error)
      else resolve()
    }))
  }
}
