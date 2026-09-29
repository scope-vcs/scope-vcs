import assert from 'node:assert/strict'
import test from 'node:test'
import { analyticsEventContext } from './client-identity'

test('event context is an immutable snapshot of runtime configuration', () => {
  const config = {
    environment: 'test' as const,
    release: 'web-first',
    token: 'phc_project',
  }
  const context = analyticsEventContext(config)
  config.release = 'web-second'

  assert.deepEqual(context, {
    environment: 'test',
    release: 'web-first',
    source: 'browser',
  })
  assert.equal(Object.isFrozen(context), true)
})
