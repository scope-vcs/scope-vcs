import assert from 'node:assert/strict'
import { Readable } from 'node:stream'
import { gunzipSync } from 'node:zlib'
import test from 'node:test'
import { launchBrowser, startFixtureServer } from '../support/fixture-server.mjs'

test('browser analytics transport uses the bounded proxy while analytics domains are blocked', async (t) => {
  const scenarios = new Map()
  let releaseCapture
  const captureReleased = new Promise(resolve => { releaseCapture = resolve })
  t.after(() => releaseCapture())
  const server = await startFixtureServer(t, 'analytics', {
    plugins: [{ name: 'scope-analytics-proxy-test', configureServer(vite) {
      vite.middlewares.use(async (incoming, outgoing, next) => {
        if (!incoming.url.startsWith('/e/')) return next()
        try {
          const scenario = scenarios.get(incoming.headers['x-scope-analytics-scenario'])
          const { analyticsEndpointResponse } = await vite.ssrLoadModule('@/server/analytics-endpoint-handler.ts')
          const request = new Request(new URL(incoming.url, scenario.runtime.SCOPE_ANALYTICS_ORIGIN), {
            method: incoming.method, headers: { ...incoming.headers, 'x-real-ip': '89.160.20.129' },
            ...(incoming.method === 'POST' ? { body: Readable.toWeb(incoming), duplex: 'half' } : {}),
          })
          const response = await analyticsEndpointResponse(request, { runtime: scenario.runtime, fetchUpstream: async (url, options) => {
            scenario.deliveries.push({ url: String(url), headers: Object.fromEntries(new Headers(options.headers)), body: Buffer.from(options.body) })
            scenario.notifyDelivery()
            await captureReleased
            return Response.json({ status: 1 })
          } })
          outgoing.writeHead(response.status, Object.fromEntries(response.headers))
          outgoing.end(Buffer.from(await response.arrayBuffer()))
        } catch (error) { next(error) }
      })
    } }],
  })
  const origin = server.resolvedUrls.local[0].replace(/\/$/, '')
  const runtime = {
    SCOPE_ANALYTICS_ENVIRONMENT: 'test', SCOPE_ANALYTICS_ORIGIN: origin,
    POSTHOG_PROJECT_TOKEN: 'phc_fixture_only', SCOPE_ANALYTICS_RELEASE: 'fixture-release',
  }
  const browser = await launchBrowser(t)
  const browserOptions = { userAgent: 'Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/145.0.0.0 Safari/537.36' }
  const { context, deliveries } = await openScenario(
    browser, browserOptions, scenarios, runtime,
  )
  const requests = []
  await context.route(/https:\/\/.*posthog\.com\//, route => route.abort('blockedbyclient'))
  await context.addCookies([{ name: 'scope_session', value: 'SECRET session', url: origin }])
  const page = await context.newPage()
  await page.clock.install()
  page.on('request', request => requests.push(request.url()))
  page.on('pageerror', error => console.log('pageerror:', error.message))
  const captureStarted = page.waitForRequest(request => new URL(request.url()).pathname === '/e/e/')
  await page.goto(origin, { waitUntil: 'domcontentloaded' })
  await captureStarted
  await page.clock.runFor(1_000)
  assert.equal(await page.evaluate(() => Boolean(window.analyticsReady)), false,
    'identity transitions must await delivery while the proxy response is held')
  releaseCapture()
  await page.waitForFunction(() => window.analyticsReady)
  assert.equal(await page.evaluate(() => window.analyticsEnabled), true, JSON.stringify(requests))
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
  await context.close()
  const geoip = await openScenario(browser, browserOptions, scenarios, {
    ...runtime, SCOPE_ANALYTICS_GEOIP: 'enabled', RAILWAY_ENVIRONMENT_ID: 'fixture',
    SCOPE_ANALYTICS_ORIGIN: origin.replace('http:', 'https:'),
  })
  const located = await geoip.context.newPage()
  await located.goto(origin, { waitUntil: 'domcontentloaded' })
  await located.waitForFunction(() => window.analyticsReady)
  assert.equal(geoip.deliveries.flatMap(decodeCapture).length, 7)
  for (const delivery of geoip.deliveries) {
    assert.equal(delivery.headers['x-forwarded-for'], '89.160.20.129')
    for (const event of decodeCapture(delivery)) {
      assert.equal(event.properties.$geoip_disable, false)
      assert.equal(event.properties.$ip, '89.160.20.129')
    }
  }
  await geoip.context.close()
  for (const preference of ['globalPrivacyControl', 'doNotTrack']) {
    const privateScenario = await openScenario(browser, browserOptions, scenarios, runtime, false)
    const privateContext = privateScenario.context
    await privateContext.addInitScript(({ preference }) => {
      Object.defineProperty(navigator, preference, { get: () => preference === 'globalPrivacyControl' ? true : '1' })
    }, { preference })
    const privatePage = await privateContext.newPage()
    await privatePage.goto(origin, { waitUntil: 'domcontentloaded' })
    await privatePage.waitForFunction(() => window.analyticsReady)
    assert.equal(privateScenario.deliveries.length, 0)
    await privateContext.close()
  }
  const stagingScenario = await openScenario(browser, browserOptions, scenarios, {
    ...runtime, SCOPE_ANALYTICS_ENVIRONMENT: 'production', RAILWAY_ENVIRONMENT_NAME: 'staging',
  })
  const staging = await stagingScenario.context.newPage()
  await staging.goto(origin, { waitUntil: 'domcontentloaded' })
  await staging.waitForFunction(() => window.analyticsReady)
  assert.equal(await staging.evaluate(() => window.analyticsEnabled), false)
  assert.equal(stagingScenario.deliveries.length, 0, 'Staging cannot use production analytics')
  await stagingScenario.context.close()
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

async function openScenario(browser, browserOptions, scenarios, runtime, deliveryEnabled = true) {
  const id = String(scenarios.size)
  const deliveries = []
  const listeners = new Set()
  scenarios.set(id, {
    runtime,
    deliveries,
    notifyDelivery: () => {
      for (const listener of listeners) listener()
      listeners.clear()
    },
  })
  const context = await browser.newContext({
    ...browserOptions,
    extraHTTPHeaders: { 'x-scope-analytics-scenario': id },
  })
  await context.addInitScript(normalVisitor)
  await context.exposeFunction('analyticsDelivered', async (count) => {
    if (!deliveryEnabled) return
    while (deliveries.flatMap(decodeCapture).length < count) {
      await new Promise(resolve => listeners.add(resolve))
    }
  })
  return { context, deliveries }
}

function normalVisitor() {
  Object.defineProperty(navigator, 'webdriver', { get: () => false })
  Object.defineProperty(navigator, 'userAgentData', { get: () => undefined })
}
