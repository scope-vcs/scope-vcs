import assert from 'node:assert/strict'
import { test } from 'node:test'
import { publicRequestOrigin } from './public-origin'

test('reports the public HTTPS origin behind Railway TLS termination', () => {
  for (const host of ['scope-web-staging.up.railway.app', 'scopevcs.com']) {
    assert.equal(
      publicRequestOrigin(`http://${host}/_serverFn/invalid`, 'railway-env'),
      `https://${host}`,
    )
  }
})

test('preserves the request scheme and port outside Railway', () => {
  assert.equal(
    publicRequestOrigin('http://localhost:3000/_serverFn/invalid', undefined),
    'http://localhost:3000',
  )
  assert.equal(publicRequestOrigin('https://scopevcs.com/path', undefined), 'https://scopevcs.com')
})
