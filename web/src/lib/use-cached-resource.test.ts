import assert from 'node:assert/strict'
import test from 'node:test'
import { resourceErrorMessage } from './use-cached-resource'

test('normalizes errors with a nonblank message or supplied fallback', () => {
  assert.equal(resourceErrorMessage(new Error('request failed'), 'fallback'), 'request failed')
  assert.equal(resourceErrorMessage(new Error('   '), 'fallback'), 'fallback')
  assert.equal(resourceErrorMessage('request failed', 'fallback'), 'fallback')
})
