import assert from 'node:assert/strict'
import { test } from 'node:test'
import { SIGN_IN_REQUIRED_HEADER, SignInRequiredError } from '../api/sign-in-required'
import { STALE_BUILD_HEADER } from '../lib/stale-build'
import { serverFunctionFailureResponse } from './server-function-failure'

test('an unknown server function becomes a plain-text stale-build conflict', async () => {
  const response = serverFunctionFailureResponse(new Error('Server function info not found for 3fdb0bfd'))
  assert.equal(response?.status, 409)
  assert.equal(response.headers.get(STALE_BUILD_HEADER), '1')
  assert.equal(response.headers.get('content-type'), 'text/plain; charset=utf-8')
  assert.equal(response.headers.get('cache-control'), 'no-store')
  assert.equal(await response.text(), 'Scope was updated. Reload to continue.')
  assert.equal(serverFunctionFailureResponse(new Error('Invalid server function ID: invalid'))?.status, 409)
  assert.equal(serverFunctionFailureResponse(new Error('database unavailable')), null)
  assert.equal(serverFunctionFailureResponse('Server function info not found for 3fdb0bfd'), null)
})

test('a missing session becomes a plain-text 401 that only its own error can produce', async () => {
  const response = serverFunctionFailureResponse(new SignInRequiredError())
  assert.equal(response?.status, 401)
  assert.equal(response.headers.get(SIGN_IN_REQUIRED_HEADER), '1')
  assert.equal(response.headers.has(STALE_BUILD_HEADER), false)
  assert.equal(response.headers.get('content-type'), 'text/plain; charset=utf-8')
  assert.equal(response.headers.get('cache-control'), 'no-store')
  assert.equal(await response.text(), 'Sign in required.')
  assert.equal(serverFunctionFailureResponse(new Error('Sign in required.')), null)
})
