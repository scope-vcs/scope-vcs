import assert from 'node:assert/strict'
import test from 'node:test'
import { DatabaseSync } from 'node:sqlite'
import { buildProductReports } from './reports.mjs'
import { posthogRequest, syncProductReports } from './sync-reports.mjs'

const report = { key: 'activation', name: 'Activation', description: 'Fixture', query: { kind: 'DataTableNode', source: { kind: 'HogQLQuery', query: 'SELECT 1' } } }

function queryReport(key, events) {
  const report = buildProductReports({
    excludedUserIds: ['scope_usr_internal'], excludedRepositoryIds: ['repoi_internal'],
  }).find((item) => item.key === key)
  const database = new DatabaseSync(':memory:')
  try {
    database.function('now', () => '2026-10-21 12:00:00')
    database.aggregate('quantile', {
      start: () => [],
      step: (values, level, value) => [...values, [level, value]],
      result: (values) => {
        const sorted = values.map(([, value]) => value).sort((left, right) => left - right)
        return sorted[Math.ceil(values[0][0] * sorted.length) - 1]
      },
    })
    database.exec('CREATE TABLE events (event TEXT, timestamp TEXT, distinct_id TEXT, properties TEXT)')
    const insert = database.prepare('INSERT INTO events VALUES (?, ?, ?, ?)')
    for (const { event, timestamp = '2026-10-20 12:00:00', distinctId = 'anonymous', ...properties } of events) {
      insert.run(event, timestamp, distinctId, JSON.stringify({ environment: 'production', ...properties }))
    }
    const sql = report.query.source.query
      .replace(/quantile\(([\d.]+)\)\(toFloat\(([^)]+)\)\)/g, 'quantile($1, $2)')
      .replace(/properties\.([A-Za-z0-9_$]+)/g, (_, name) => `json_extract(properties, '$.${name}')`)
      .replace(/now\(\) - INTERVAL (\d+) DAY/g, "datetime(now(), '-$1 days')")
    return database.prepare(sql).all().map((row) => ({ ...row }))
  } finally {
    database.close()
  }
}

test('browser geography counts scoped pageviews and retains missing location coverage', () => {
  const pageview = ({ event = '$pageview', ...properties } = {}) => ({
    event, source: 'browser',
    $geoip_country_name: 'United States', $geoip_subdivision_1_name: 'Texas', $geoip_city_name: 'Austin',
    ...properties,
  })
  assert.deepEqual(queryReport('browser-geography', [
    pageview(),
    pageview(),
    pageview({ timestamp: '2026-09-21 12:00:00' }),
    pageview({ $geoip_city_name: null }),
    pageview({ $geoip_country_name: 'Germany', $geoip_subdivision_1_name: 'Berlin', $geoip_city_name: 'Berlin' }),
    pageview({ $geoip_country_name: null, $geoip_subdivision_1_name: null, $geoip_city_name: null }),
    pageview({ $geoip_country_name: '', $geoip_subdivision_1_name: '', $geoip_city_name: '' }),
    pageview({ $geoip_country_name: undefined, $geoip_subdivision_1_name: undefined, $geoip_city_name: undefined }),
    pageview({ environment: 'test' }),
    pageview({ source: 'server' }),
    pageview({ event: 'frontend_error' }),
    pageview({ timestamp: '2026-09-21 11:59:59' }),
    pageview({ distinctId: 'scope_usr_internal' }),
    pageview({ repository_id: 'repoi_internal' }),
  ]), [
    { country: 'United States', region: 'Texas', city: 'Austin', pageviews: 3 },
    { country: 'Unknown', region: 'Unknown', city: 'Unknown', pageviews: 3 },
    { country: 'Germany', region: 'Berlin', city: 'Berlin', pageviews: 1 },
    { country: 'United States', region: 'Texas', city: 'Unknown', pageviews: 1 },
  ])
})

test('run log timing reports percentiles and stored share per trigger and provider for the last week', () => {
  const timing = (trigger, provider, durationMs, source = 'stored', properties = {}) => ({
    event: 'run_log_timing', trigger, provider, duration_ms: durationMs, log_source: source, ...properties,
  })
  assert.deepEqual(queryReport('run-log-timing', [
    ...[100, 200, 300, 400].map((duration, index) => timing('job_click', 'github', duration, index === 0 ? 'fetched' : 'stored')),
    timing('page_load', 'native', 900),
    timing('job_click', 'github', 9000, 'stored', { timestamp: '2026-10-14 11:59:59' }),
    timing('job_click', 'github', 9000, 'stored', { environment: 'test' }),
    timing('job_click', 'github', 9000, 'stored', { event: 'web_vital' }),
  ]), [
    { trigger: 'job_click', provider: 'github', samples: 4, p50_ms: 200, p75_ms: 300, p95_ms: 400, stored_percent: 75 },
    { trigger: 'page_load', provider: 'native', samples: 1, p50_ms: 900, p75_ms: 900, p95_ms: 900, stored_percent: 100 },
  ])
})

test('invalid queries stop publication before any dashboard or insight mutation', async () => {
  const calls = []
  const request = async (path) => { calls.push(path); throw new Error('invalid query') }
  await assert.rejects(syncProductReports(request, [report]), /invalid query/)
  assert.deepEqual(calls, ['query/'])
})

test('sync reuses tagged insights across pagination and preserves other dashboard associations', async () => {
  const calls = []
  const request = async (path, { method = 'GET', body } = {}) => {
    calls.push({ path, method, body })
    if (method === 'POST' && path === 'query/') return { results: [] }
    if (method === 'GET' && path === 'dashboards/') return { results: [], next: 'dashboards/?offset=1' }
    if (method === 'GET' && path === 'dashboards/?offset=1') return { results: [{ id: 9, name: 'Scope product outcomes' }], next: null }
    if (method === 'GET' && path === 'insights/?dashboards=%5B9%5D') return { results: [{ id: 12, tags: ['mine', 'scope-product-report:activation'], dashboards: [4, 9] }], next: null }
    if (method === 'PATCH' && path === 'insights/12/') return { id: 12 }
    throw new Error(`Unexpected ${method} ${path}`)
  }
  assert.equal((await syncProductReports(request, [report])).dashboardId, 9)
  const writes = calls.filter((call) => call.method === 'PATCH')
  assert.equal(writes.length, 1)
  assert.deepEqual(writes[0].body.dashboards, [4, 9])
  assert.deepEqual(writes[0].body.tags, ['mine', 'scope-product-report:activation'])
  assert.equal(calls.some((call) => call.method === 'POST' && call.path !== 'query/'), false)
})

test('missing reports create one dashboard and attach the new insight', async () => {
  const calls = []
  const request = async (path, { method = 'GET', body } = {}) => {
    calls.push({ path, method })
    if (method === 'POST' && path === 'query/') {
      assert.deepEqual(body.query, report.query.source)
      assert.equal(body.refresh, 'blocking')
      return { results: [] }
    }
    if (method === 'GET' && ['dashboards/', 'insights/?dashboards=%5B2%5D'].includes(path)) return { results: [], next: null }
    if (method === 'POST' && path === 'dashboards/') {
      assert.equal(body.name, 'Scope product outcomes')
      return { id: 2 }
    }
    if (method === 'POST' && path === 'insights/') {
      assert.deepEqual(body, {
        name: report.name, description: report.description, query: report.query,
        tags: ['scope-product-report:activation'], dashboards: [2],
      })
      return { id: 3, short_id: 'abc' }
    }
    throw new Error(`Unexpected ${method} ${path}`)
  }
  assert.deepEqual(await syncProductReports(request, [report]), {
    dashboardId: 2, insights: [{ key: 'activation', id: 3, shortId: 'abc' }],
  })
  assert.deepEqual(calls, [
    { path: 'query/', method: 'POST' },
    { path: 'dashboards/', method: 'GET' },
    { path: 'dashboards/', method: 'POST' },
    { path: 'insights/?dashboards=%5B2%5D', method: 'GET' },
    { path: 'insights/', method: 'POST' },
  ])
})

test('API transport refuses foreign pagination and reports upstream failure without leaking a body', async () => {
  const requests = []
  const request = posthogRequest({ apiKey: 'fixture-only', projectId: '123', fetchImpl: async (url, options) => {
    requests.push({ url, options })
    return new Response('private detail', { status: 403 })
  } })
  await assert.rejects(request('https://other.example/api/projects/123/'), /Unexpected/)
  await assert.rejects(request('../456/insights/'), /Unexpected/)
  assert.equal(requests.length, 0)
  await assert.rejects(request('insights/'), (error) => error.message.endsWith('HTTP 403') && !error.message.includes('private detail'))
  assert.equal(requests[0].options.redirect, 'error')
})

test('cohort exclusions reject SQL fragments and apply to every query before aggregation', () => {
  assert.throws(() => buildProductReports({ excludedUserIds: ["scope_usr_x' OR 1=1"] }), /Invalid/)
  const reports = buildProductReports({ excludedUserIds: ['scope_usr_internal'], excludedRepositoryIds: ['repoi_internal'] })
  for (const item of reports) {
    assert.match(item.query.source.query, /properties.environment = 'production'/)
    assert.match(item.query.source.query, /distinct_id NOT IN \('scope_usr_internal'\)/)
    assert.match(item.query.source.query, /coalesce\(properties.repository_id, ''\) NOT IN \('repoi_internal'\)/)
  }
  const completion = reports.find((item) => item.key === 'request-completion').query.source.query
  assert.match(completion, /s.request_id = e.properties.request_id/)
  const runs = reports.find((item) => item.key === 'workflow-runs').query.source.query
  assert.match(runs, /notEmpty\(properties.run_result\)/)
})
