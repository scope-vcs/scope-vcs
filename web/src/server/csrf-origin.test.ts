import assert from 'node:assert/strict'
import { test } from 'node:test'
import { matchesCsrfOrigin } from './csrf-origin'

test('matches the public HTTPS origin behind Railway TLS termination', () => {
  for (const host of ['scope-web-staging.up.railway.app', 'scopevcs.com']) {
    const requestUrl = `http://${host}/_serverFn/invalid`
    assert.equal(matchesCsrfOrigin(`https://${host}`, requestUrl, 'railway-env'), true)
    for (const origin of [
      `http://${host}`,
      `https://${host}.another.example`,
      `https://${host}:444`,
      `https://user:password@${host}`,
      'https://another.example',
      'null',
      '',
    ]) {
      assert.equal(matchesCsrfOrigin(origin, requestUrl, 'railway-env'), false, origin)
    }
  }
})

test('preserves the request scheme and port outside Railway', () => {
  const requestUrl = 'http://localhost:3000/_serverFn/invalid'
  assert.equal(matchesCsrfOrigin('http://localhost:3000', requestUrl, undefined), true)
  assert.equal(matchesCsrfOrigin('https://localhost:3000', requestUrl, undefined), false)
  assert.equal(matchesCsrfOrigin('http://localhost', requestUrl, undefined), false)
  assert.equal(matchesCsrfOrigin('https://scopevcs.com', 'https://scopevcs.com/path', undefined), true)
})
