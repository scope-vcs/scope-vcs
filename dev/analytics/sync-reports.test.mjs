import assert from 'node:assert/strict'
import test from 'node:test'
import { buildProductReports } from './reports.mjs'
import { posthogRequest, syncProductReports } from './sync-reports.mjs'

const report = { key: 'activation', name: 'Activation', description: 'Fixture', query: { kind: 'DataTableNode', source: { kind: 'HogQLQuery', query: 'SELECT 1' } } }

test('invalid queries stop publication before any dashboard or insight mutation', async () => {
  const calls = []
  const request = async (path) => { calls.push(path); throw new Error('invalid query') }
  await assert.rejects(syncProductReports(request, [report]), /invalid query/)
  assert.deepEqual(calls, ['query/'])
})

test('sync reuses tagged insights across pagination and preserves other dashboard associations', async () => {
  const calls = []
  const request = async (path, options = {}) => {
    calls.push({ path, ...options })
    if (path === 'query/') return { results: [] }
    if (path === 'dashboards/') return { results: [], next: 'dashboards/?offset=1' }
    if (path === 'dashboards/?offset=1') return { results: [{ id: 9, name: 'Scope product outcomes' }], next: null }
    if (path === 'insights/?dashboards=%5B9%5D') return { results: [{ id: 12, tags: ['mine', 'scope-product-report:activation'], dashboards: [4, 9] }], next: null }
    if (path === 'insights/12/') return { id: 12 }
    throw new Error(`Unexpected ${path}`)
  }
  for (let run = 0; run < 2; run++) assert.equal((await syncProductReports(request, [report])).dashboardId, 9)
  const writes = calls.filter((call) => call.method === 'PATCH')
  assert.equal(writes.length, 2)
  assert.deepEqual(writes[0].body.dashboards, [4, 9])
  assert.deepEqual(writes[0].body.tags, ['mine', 'scope-product-report:activation'])
  assert.equal(calls.some((call) => call.method === 'POST' && call.path !== 'query/'), false)
})

test('missing reports create one dashboard and attach the new insight', async () => {
  const request = async (path, options = {}) => {
    if (path === 'query/') return { results: [] }
    if (options.method === 'GET' || !options.method) return { results: [], next: null }
    if (path === 'dashboards/') return { id: 2 }
    assert.deepEqual(options.body.dashboards, [2])
    return { id: 3, short_id: 'abc' }
  }
  assert.deepEqual(await syncProductReports(request, [report]), {
    dashboardId: 2, insights: [{ key: 'activation', id: 3, shortId: 'abc' }],
  })
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
