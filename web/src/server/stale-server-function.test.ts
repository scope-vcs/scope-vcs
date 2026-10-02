import assert from 'node:assert/strict'
import { test } from 'node:test'
import { STALE_BUILD_HEADER } from '../lib/stale-build'
import { staleServerFunctionResponse } from './stale-server-function'

test('an unknown server function becomes a plain-text stale-build conflict', async () => {
  const response = staleServerFunctionResponse(new Error('Server function info not found for 3fdb0bfd'))
  assert.equal(response?.status, 409)
  assert.equal(response.headers.get(STALE_BUILD_HEADER), '1')
  assert.equal(response.headers.get('content-type'), 'text/plain; charset=utf-8')
  assert.equal(response.headers.get('cache-control'), 'no-store')
  assert.equal(await response.text(), 'Scope was updated. Reload to continue.')
  assert.equal(staleServerFunctionResponse(new Error('Invalid server function ID: invalid'))?.status, 409)
  assert.equal(staleServerFunctionResponse(new Error('database unavailable')), null)
  assert.equal(staleServerFunctionResponse('Server function info not found for 3fdb0bfd'), null)
})
