import assert from 'node:assert/strict'
import { afterEach, test } from 'node:test'

import { readinessResponse } from './readiness'

const originalApiUrl = process.env.SCOPE_API_INTERNAL_URL
const originalNodeEnv = process.env.NODE_ENV

afterEach(() => {
  restoreEnv('SCOPE_API_INTERNAL_URL', originalApiUrl)
  restoreEnv('NODE_ENV', originalNodeEnv)
})

test('reports ready after an unauthenticated internal API readiness check', async () => {
  process.env.SCOPE_API_INTERNAL_URL = 'http://scope-api.railway.internal:8080/'
  const response = await readinessResponse(async (input, init) => {
    assert.equal(input, 'http://scope-api.railway.internal:8080/readyz')
    assert.equal(init?.method, 'GET')
    assert.equal(init?.redirect, 'error')
    assert.equal(new Headers(init?.headers).has('authorization'), false)
    assert.ok(init?.signal)
    return Response.json({ status: 'ok' })
  })

  assert.equal(response.status, 200)
  assert.equal(response.headers.get('cache-control'), 'no-store')
  assert.deepEqual(await response.json(), {
    status: 'ok',
    service: 'web',
    checks: [{ name: 'api', status: 'ok' }],
  })
})

test('reports unavailable without exposing upstream response or connection details', async () => {
  process.env.SCOPE_API_INTERNAL_URL = 'http://secret-api-host:8080'
  const response = await readinessResponse(async () => new Response(
    'secret database failure',
    { status: 503 },
  ))

  assert.equal(response.status, 503)
  const body = await response.text()
  assert.doesNotMatch(body, /secret|database|host/i)
  assert.deepEqual(JSON.parse(body), {
    status: 'unavailable',
    service: 'web',
    checks: [{ name: 'api', status: 'unavailable' }],
  })
})

test('reports unavailable when API configuration is missing', async () => {
  delete process.env.SCOPE_API_INTERNAL_URL
  process.env.NODE_ENV = 'production'

  const response = await readinessResponse(async () => {
    throw new Error('fetch should not run without an API connection')
  })

  assert.equal(response.status, 503)
  assert.doesNotMatch(await response.text(), /SCOPE_API_INTERNAL_URL/)
})

test('bounds an API readiness check that does not complete', async () => {
  process.env.SCOPE_API_INTERNAL_URL = 'http://scope-api.railway.internal:8080'
  const response = await readinessResponse((_input, init) => new Promise(
    (_resolve, reject) => {
      init?.signal?.addEventListener(
        'abort',
        () => reject(init.signal?.reason),
        { once: true },
      )
    },
  ), 5)

  assert.equal(response.status, 503)
})

function restoreEnv(name: string, value: string | undefined) {
  if (value === undefined) {
    delete process.env[name]
  } else {
    process.env[name] = value
  }
}
