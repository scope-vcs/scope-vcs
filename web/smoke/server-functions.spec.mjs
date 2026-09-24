import assert from 'node:assert/strict'
import { existsSync } from 'node:fs'
import { test } from 'node:test'
import { builtServerFunctions, serverFunctionName } from './server-functions-smoke.mjs'

const request = (path) => ({ url: () => `https://scope.example${path}` })
const hasManifest = process.env.SCOPE_SMOKE_SERVER_MANIFEST ||
  existsSync(new URL('../.output/server/_ssr/ssr.mjs', import.meta.url))

test('server function interception recognizes production IDs from the built manifest', {
  skip: !hasManifest && process.env.SCOPE_REQUIRE_BUILT_MANIFEST !== '1'
    ? 'No local build; pnpm build runs this check against its emitted manifest'
    : false,
}, () => {
  const compiledFunctions = builtServerFunctions()
  assert.ok(compiledFunctions.size > 0, 'No server function manifest entries found in the built server')
  for (const [id, handler] of compiledFunctions) {
    assert.match(id, /^[0-9a-f]{64}$/)
    assert.match(handler, /_createServerFn_handler$/)
    assert.equal(serverFunctionName(request(`/_serverFn/${id}?payload=test`)), handler)
  }
})

test('server function interception also decodes the development compiler format', () => {
  const id = Buffer.from(JSON.stringify({
    file: 'src/routes/$owner.$repo._code.index.tsx',
    export: 'loadRepoFile_createServerFn_handler',
  })).toString('base64url')
  assert.equal(serverFunctionName(request(`/_serverFn/${id}?payload=test`)),
    'loadRepoFile_createServerFn_handler')
})

test('server function interception rejects unknown IDs and ignores other endpoints', () => {
  for (const id of ['f'.repeat(64), '', 'invalid', Buffer.from('{}').toString('base64url')]) {
    assert.throws(() => serverFunctionName(request(`/_serverFn/${id}`)), /Unknown server function ID/)
  }
  assert.equal(serverFunctionName(request('/assets/app.js')), '')
})
