import assert from 'node:assert/strict'
import { Readable } from 'node:stream'
import { gunzipSync } from 'node:zlib'
import test from 'node:test'
import { launchBrowser, startFixtureServer } from './support/fixture-server.mjs'

test('browser analytics transport uses the bounded proxy while analytics domains are blocked', async (t) => {
  let runtime
  const deliveries = []
  const server = await startFixtureServer(t, 'analytics', {
    plugins: [{ name: 'scope-analytics-proxy-test', configureServer(vite) {
      vite.middlewares.use(async (incoming, outgoing, next) => {
        if (!incoming.url.startsWith('/e/')) return next()
        try {
          const { analyticsEndpointResponse } = await vite.ssrLoadModule('@/server/analytics-endpoint-handler.ts')
          const request = new Request(new URL(incoming.url, runtime.SCOPE_ANALYTICS_ORIGIN), {
            method: incoming.method, headers: { ...incoming.headers, 'x-real-ip': '89.160.20.129' },
            ...(incoming.method === 'POST' ? { body: Readable.toWeb(incoming), duplex: 'half' } : {}),
          })
          const response = await analyticsEndpointResponse(request, { runtime, fetchUpstream: async (url, options) => {
            deliveries.push({ url: String(url), headers: Object.fromEntries(new Headers(options.headers)), body: Buffer.from(options.body) })
            return Response.json({ status: 1 })
          } })
          outgoing.writeHead(response.status, Object.fromEntries(response.headers))
          outgoing.end(Buffer.from(await response.arrayBuffer()))
        } catch (error) { next(error) }
      })
    } }],
  })
  const origin = server.resolvedUrls.local[0].replace(/\/$/, '')
  runtime = {
    SCOPE_ANALYTICS_ENVIRONMENT: 'test', SCOPE_ANALYTICS_ORIGIN: origin,
    POSTHOG_PROJECT_TOKEN: 'phc_fixture_only', SCOPE_ANALYTICS_RELEASE: 'fixture-release',
  }
  const browser = await launchBrowser(t)
  const browserOptions = { userAgent: 'Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/145.0.0.0 Safari/537.36' }
  const context = await browser.newContext(browserOptions)
  await context.addInitScript(normalVisitor)
  const requests = []
  await context.route(/https:\/\/.*posthog\.com\//, route => route.abort('blockedbyclient'))
  await context.addCookies([{ name: 'scope_session', value: 'SECRET session', url: origin }])
  const page = await context.newPage()
  page.on('request', request => requests.push(request.url()))
  page.on('pageerror', error => console.log('pageerror:', error.message))
  await page.goto(origin, { waitUntil: 'domcontentloaded' })
  await page.waitForFunction(() => window.analyticsReady)
  assert.equal(await page.evaluate(() => window.analyticsEnabled), true, JSON.stringify(requests))
  await waitFor(() => deliveries.flatMap(decodeCapture).length >= 7)
  assert.equal(await page.evaluate(() => localStorage.length), 0, 'analytics must not store identities')
  const events = deliveries.flatMap(decodeCapture)
  assert.deepEqual(events.map(event => event.event).sort(), [
    '$identify', '$identify',
    '$pageview', '$pageview', '$pageview',
    'frontend_error', 'frontend_error',
  ].sort())
  assert.ok(requests.some(url => new URL(url).pathname === '/e/e/'))
  assert.equal(requests.some(url => new URL(url).hostname.endsWith('posthog.com')), false)
  for (const delivery of deliveries) {
    assert.equal(new URL(delivery.url).origin, 'https://us.i.posthog.com')
    assert.equal(new URL(delivery.url).pathname, '/e/')
    assert.equal(delivery.headers.cookie, undefined)
    assert.equal(delivery.headers.referer, undefined)
    assert.equal(delivery.headers.authorization, undefined)
  }
  for (const event of events) {
    assert.equal(event.properties.environment, 'test')
    assert.equal(event.properties.release, 'fixture-release')
    assert.equal(event.properties.source, 'browser')
    assert.equal(event.properties.$geoip_disable, true)
    assert.equal(JSON.stringify(event).includes('SECRET'), false)
    assert.equal(JSON.stringify(event).includes('private-repository'), false)
  }
  const pageViews = events.filter(event => event.event === '$pageview')
  const identifies = events.filter(event => event.event === '$identify')
  const diagnostics = events.filter(event => event.event === 'frontend_error')
  const anonymousPageViews = pageViews.filter(
    event => !event.properties.distinct_id.startsWith('scope_usr_'),
  )
  assert.equal(anonymousPageViews.length, 2)
  assert.equal(new Set(
    anonymousPageViews.map(event => event.properties.distinct_id),
  ).size, 2)
  assert.equal(pageViews.filter(
    event => event.properties.distinct_id === 'scope_usr_two',
  ).length, 1)
  assert.equal(identifies.filter(
    event => event.properties.distinct_id === 'scope_usr_one',
  ).length, 2)
  assert.equal(identifies.filter(
    event => event.properties.distinct_id === 'scope_usr_two',
  ).length, 0, 'switching accounts resets, leaving no anonymous events to merge')
  assert.equal(diagnostics.find(
    event => event.properties.error_origin === 'window',
  ).properties.distinct_id, 'scope_usr_one')
  assert.equal(diagnostics.find(
    event => event.properties.error_origin === 'route',
  ).properties.distinct_id, 'scope_usr_two')
  runtime = { ...runtime, SCOPE_ANALYTICS_GEOIP: 'enabled', RAILWAY_ENVIRONMENT_ID: 'fixture',
    SCOPE_ANALYTICS_ORIGIN: origin.replace('http:', 'https:') }
  const beforeGeoip = deliveries.length
  const located = await browser.newPage()
  await located.goto(origin, { waitUntil: 'domcontentloaded' })
  await located.waitForFunction(() => window.analyticsReady)
  await waitFor(() => deliveries.length >= beforeGeoip + 7)
  for (const delivery of deliveries.slice(beforeGeoip)) {
    assert.equal(delivery.headers['x-forwarded-for'], '89.160.20.129')
    for (const event of decodeCapture(delivery)) {
      assert.equal(event.properties.$geoip_disable, false)
      assert.equal(event.properties.$ip, '89.160.20.129')
    }
  }
  for (const preference of ['globalPrivacyControl', 'doNotTrack']) {
    const privateContext = await browser.newContext(browserOptions)
    await privateContext.addInitScript(({ preference }) => {
      Object.defineProperty(navigator, preference, { get: () => preference === 'globalPrivacyControl' ? true : '1' })
    }, { preference })
    const privatePage = await privateContext.newPage()
    const beforePrivate = deliveries.length
    await privatePage.goto(origin, { waitUntil: 'domcontentloaded' })
    await privatePage.waitForFunction(() => window.analyticsReady)
    assert.equal(deliveries.length, beforePrivate)
    await privateContext.close()
  }
  const beforeStaging = deliveries.length
  runtime = { ...runtime, SCOPE_ANALYTICS_ENVIRONMENT: 'production', RAILWAY_ENVIRONMENT_NAME: 'staging' }
  const staging = await browser.newPage()
  await staging.goto(origin, { waitUntil: 'domcontentloaded' })
  await staging.waitForFunction(() => window.analyticsReady)
  assert.equal(await staging.evaluate(() => window.analyticsEnabled), false)
  assert.equal(deliveries.length, beforeStaging, 'Staging cannot use production analytics')
})

function decodeCapture(delivery) {
  const query = new URL(delivery.url).searchParams
  let body = delivery.body
  if (delivery.headers['content-encoding'] === 'gzip' || query.get('compression') === 'gzip-js' || (body[0] === 0x1f && body[1] === 0x8b)) body = gunzipSync(body)
  const raw = body.toString()
  const parsed = raw.startsWith('data=')
    ? JSON.parse(Buffer.from(new URLSearchParams(raw).get('data'), 'base64').toString())
    : JSON.parse(raw)
  return Array.isArray(parsed) ? parsed : parsed.batch ?? [parsed]
}

async function waitFor(condition) {
  const deadline = Date.now() + 15000
  while (!condition()) {
    assert.ok(Date.now() < deadline, 'analytics transport did not deliver its queued events')
    await new Promise(resolve => setTimeout(resolve, 50))
  }
}

function normalVisitor() {
  Object.defineProperty(navigator, 'webdriver', { get: () => false })
  Object.defineProperty(navigator, 'userAgentData', { get: () => undefined })
}
