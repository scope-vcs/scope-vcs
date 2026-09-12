import assert from 'node:assert/strict'
import { existsSync, readFileSync } from 'node:fs'
import { test } from 'node:test'
import ts from 'typescript'
import { productionFunctions, serverFunctionName } from './server-functions-smoke.mjs'

const request = (path) => ({ url: () => `https://scope.example${path}` })
const serverBundle = new URL('../.output/server/_ssr/ssr.mjs', import.meta.url)

test('server function interception recognizes production IDs from the built manifest', {
  skip: !existsSync(serverBundle) && process.env.SCOPE_REQUIRE_BUILT_MANIFEST !== '1'
    ? 'No local build; pnpm build runs this check against its emitted manifest'
    : false,
}, () => {
  const source = ts.createSourceFile('ssr.mjs', readFileSync(serverBundle, 'utf8'), ts.ScriptTarget.Latest, true, ts.ScriptKind.JS)
  const manifest = new Map()
  ts.forEachChild(source, function visit(node) {
    if (ts.isPropertyAssignment(node) && ts.isStringLiteral(node.name) && ts.isObjectLiteralExpression(node.initializer)) {
      const name = node.initializer.properties.find((property) =>
        ts.isPropertyAssignment(property) && property.name.text === 'functionName')
      if (name && ts.isStringLiteral(name.initializer)) {
        manifest.set(node.name.text, name.initializer.text)
      }
    }
    ts.forEachChild(node, visit)
  })

  assert.ok(manifest.size > 0, 'No server function manifest entries found in the built server')
  for (const [id, handler] of productionFunctions) {
    assert.equal(manifest.get(id), handler, `Smoke interception for ${handler} disagrees with the built manifest`)
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
