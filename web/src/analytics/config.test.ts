import assert from 'node:assert/strict'
import test from 'node:test'
import { parseAnalyticsRuntimeConfig } from './config'

test('accepts enabled and disabled runtime analytics configuration', () => {
  assert.equal(parseAnalyticsRuntimeConfig(null), null)
  assert.deepEqual(parseAnalyticsRuntimeConfig({
    environment: 'production',
    release: 'abc123',
    token: 'phc_public',
  }), {
    environment: 'production',
    release: 'abc123',
    token: 'phc_public',
  })
})

test('fails closed for malformed runtime analytics configuration', () => {
  assert.equal(parseAnalyticsRuntimeConfig({
    environment: 'staging',
    release: null,
    token: 'phc_public',
  }), null)
  assert.equal(parseAnalyticsRuntimeConfig({
    environment: 'test',
    release: 123,
    token: 'phc_public',
  }), null)
  assert.equal(parseAnalyticsRuntimeConfig({
    environment: 'test',
    release: null,
    token: '',
  }), null)
})
