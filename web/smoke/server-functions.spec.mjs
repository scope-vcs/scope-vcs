import assert from 'node:assert/strict'
import { test } from 'node:test'
import { serverFunctionName } from './server-functions-smoke.mjs'

const request = (path) => ({ url: () => `https://scope.example${path}` })

test('server function interception recognizes production IDs from the built manifest', () => {
  assert.equal(serverFunctionName(request('/_serverFn/1a244e708e6aee5e00cbc325738360030b4eda0f97faea6b1885eadfcbe133d1?payload=test')),
    'loadChangesPage_createServerFn_handler')
  assert.equal(serverFunctionName(request('/_serverFn/02b6cf9544857f2eab3a686af0c993ec5e022be826c7062faa513d1139d206a4')),
    'loadRequestQueuePage_createServerFn_handler')
})

test('attachment metadata requests use the production request-page ID', () => {
  assert.equal(serverFunctionName(request('/_serverFn/5bbd567bcd27c0c8b8cddec5cf03ca8ce15a58fcde87e8af3a0e0509bde421b7')),
    'listRequestAttachments_createServerFn_handler')
})

test('server function interception also decodes the development compiler format', () => {
  const id = Buffer.from(JSON.stringify({
    file: 'src/routes/$owner.$repo._code.index.tsx',
    export: 'loadRepoFile_createServerFn_handler',
  })).toString('base64url')
  assert.equal(serverFunctionName(request(`/_serverFn/${id}?payload=test`)),
    'loadRepoFile_createServerFn_handler')
})

test('server function interception rejects unknown or malformed IDs', () => {
  for (const id of ['f'.repeat(64), '', 'invalid', Buffer.from('{}').toString('base64url')]) {
    assert.throws(() => serverFunctionName(request(`/_serverFn/${id}`)), /Unknown server function ID/)
  }
})

test('response predicates ignore requests outside the server function endpoint', () => {
  assert.equal(serverFunctionName(request('/assets/app.js')), '')
  assert.equal(serverFunctionName(request('/v1/repos/dev/public-demo/events')), '')
})
