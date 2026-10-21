import assert from 'node:assert/strict'
import { readFile } from 'node:fs/promises'
import { spawnSync } from 'node:child_process'
import test from 'node:test'
import { configureGeoip } from './configure-geoip.mjs'

const hog = await readFile(new URL('./geoip.hog', import.meta.url), 'utf8')
const current = {
  id: 'geoip-1', template: { id: 'template-geoip' }, name: 'Scope approximate location',
  hog, enabled: true, updated_at: '2026-10-21T12:00:00Z', execution_order: 2,
  draft: null, draft_updated_at: null,
}

function fixture({ project = {}, transformation = {}, pages, verifiedProject = {}, verifiedFunction = {}, failProjectWrite = false } = {}) {
  const calls = []
  let savedProject = { id: 123, anonymize_ips: true, ...project }
  let savedFunction = { ...current, ...transformation }
  const request = async (path, { method = 'GET', body } = {}) => {
    calls.push({ path, method, body })
    if (method === 'GET' && path === '') return { ...savedProject, ...(calls.length > 1 ? verifiedProject : {}) }
    if (method === 'GET' && path.startsWith('hog_functions/?')) {
      return pages?.[path] ?? { results: [savedFunction], next: null }
    }
    if (method === 'GET' && path === 'hog_functions/geoip-1/') return { ...savedFunction, ...(calls.some(({ method }) => method === 'PATCH') ? verifiedFunction : {}) }
    if (method === 'PATCH' && path === '') {
      if (failProjectWrite) throw new Error('project write rejected')
      savedProject = { ...savedProject, ...body }
      return savedProject
    }
    if (method === 'PATCH' && path === 'hog_functions/geoip-1/') {
      savedFunction = { ...savedFunction, ...body }
      return savedFunction
    }
    throw new Error(`Unexpected ${method} ${path}`)
  }
  return { request, calls }
}

test('apply paginates discovery, anonymizes first, and preserves transformation order with concurrency protection', async () => {
  const existing = { ...current, name: 'GeoIP', hog: 'return event', enabled: false }
  const { request, calls } = fixture({ project: { anonymize_ips: false }, transformation: existing, pages: {
    'hog_functions/?type=transformation&limit=1000': { results: [{ id: 'other', template: { id: 'other' } }], next: 'hog_functions/?offset=1000' },
    'hog_functions/?offset=1000': { results: [existing], next: null },
  } })
  assert.equal((await configureGeoip(request, hog, { apply: true })).status, 'verified')
  assert.deepEqual(calls.map(({ method, path }) => [method, path]), [
    ['GET', ''], ['GET', 'hog_functions/?type=transformation&limit=1000'],
    ['GET', 'hog_functions/?offset=1000'], ['GET', 'hog_functions/geoip-1/'],
    ['PATCH', ''], ['PATCH', 'hog_functions/geoip-1/'],
    ['GET', ''], ['GET', 'hog_functions/geoip-1/'],
  ])
  assert.deepEqual(calls[4].body, { anonymize_ips: true })
  assert.deepEqual(calls[5].body, {
    hog, name: 'Scope approximate location', enabled: true, execution_order: 2, base_updated_at: existing.updated_at,
  })
})

test('missing, ambiguous, cyclic, and pending-draft discovery refuse all mutations', async () => {
  const first = 'hog_functions/?type=transformation&limit=1000'
  const scenarios = [
    { pages: { [first]: { results: [], next: null } } },
    { pages: { [first]: { results: [{ id: current.id, template: current.template }], next: 'hog_functions/?offset=1' }, 'hog_functions/?offset=1': { results: [{ ...current, id: 'second' }], next: null } } },
    { pages: { [first]: { results: [{ id: current.id, template: current.template }], next: first } } },
    { transformation: { draft_updated_at: '2026-10-21T13:00:00Z' } },
    { transformation: { hog: 'old', updated_at: null } },
  ]
  for (const scenario of scenarios) {
    const { request, calls } = fixture({ project: { anonymize_ips: false }, ...scenario })
    await assert.rejects(configureGeoip(request, hog, { apply: true }), /exactly one|pagination cycle|pending draft|concurrency timestamp/)
    assert.equal(calls.every(({ method }) => method === 'GET'), true)
  }
})

test('preview reports changes without mutation and applying twice does not repeat writes', async () => {
  const { request, calls } = fixture({ project: { anonymize_ips: false }, transformation: { hog: 'old' } })
  assert.deepEqual(await configureGeoip(request, hog), {
    projectId: 123, functionId: 'geoip-1', anonymizeIps: 'update-required', transformation: 'update-required', status: 'preview',
  })
  assert.equal(calls.every(({ method }) => method === 'GET'), true)
  await configureGeoip(request, hog, { apply: true })
  calls.length = 0
  assert.equal((await configureGeoip(request, hog, { apply: true })).status, 'verified')
  assert.equal(calls.length, 5)
  assert.equal(calls.every(({ method }) => method === 'GET'), true)
})

test('project write failure prevents changing the transformation', async () => {
  const { request, calls } = fixture({ project: { anonymize_ips: false }, transformation: { hog: 'old' }, failProjectWrite: true })
  await assert.rejects(configureGeoip(request, hog, { apply: true }), /project write rejected/)
  assert.deepEqual(calls.filter(({ method }) => method !== 'GET').map(({ path }) => path), [''])
})

test('apply rejects stale live settings and staged-only updates during verification', async () => {
  const scenarios = [
    { verifiedProject: { anonymize_ips: false } },
    { verifiedFunction: { hog: 'old' } },
    { verifiedFunction: { enabled: false } },
    { verifiedFunction: { draft: { hog } } },
    { verifiedFunction: { draft_updated_at: '2026-10-21T13:00:00Z' } },
    { verifiedFunction: { execution_order: 9 } },
  ]
  for (const scenario of scenarios) {
    const { request } = fixture({ project: { anonymize_ips: false }, transformation: { hog: 'old' }, ...scenario })
    await assert.rejects(configureGeoip(request, hog, { apply: true }), /verification failed/)
  }
})

test('operator preview uses the shared API transport and prints only IDs and statuses', () => {
  const bootstrap = `globalThis.fetch = async (url, options) => {
    if (options.method !== 'GET') throw new Error('Preview attempted a write')
    const body = url.pathname.endsWith('/hog_functions/geoip-1/')
      ? ${JSON.stringify(current)}
      : url.pathname.endsWith('/hog_functions/')
      ? { results: [${JSON.stringify(current)}], next: null }
      : { id: 123, anonymize_ips: false, api_token: 'private-project-token' }
    return Response.json(body)
  }`
  const result = spawnSync(process.execPath, ['--import', `data:text/javascript,${encodeURIComponent(bootstrap)}`,
    'dev/analytics/configure-geoip.mjs', '--preview'], {
    encoding: 'utf8', env: { ...process.env, POSTHOG_PROJECT_ID: '123', POSTHOG_PERSONAL_API_KEY: 'fixture-key', POSTHOG_APP_HOST: 'https://us.posthog.com' },
  })
  assert.equal(result.status, 0, result.stderr)
  assert.deepEqual(JSON.parse(result.stdout), {
    projectId: 123, functionId: 'geoip-1', anonymizeIps: 'update-required', transformation: 'current', status: 'preview',
  })
})
