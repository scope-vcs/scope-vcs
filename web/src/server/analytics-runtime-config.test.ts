import assert from 'node:assert/strict'
import test from 'node:test'
import { getAnalyticsRuntimeConfig } from './analytics-runtime-config'

const enabled = {
  POSTHOG_PROJECT_TOKEN: ' phc_public ',
  SCOPE_ANALYTICS_ENVIRONMENT: 'production',
  SCOPE_ANALYTICS_ORIGIN: 'https://scopevcs.com',
  SCOPE_ANALYTICS_RELEASE: ' abc123 ',
}

test('returns safe public configuration only at the configured origin', () => {
  assert.deepEqual(
    getAnalyticsRuntimeConfig('https://scopevcs.com/e/config', enabled),
    { environment: 'production', release: 'abc123', token: 'phc_public' },
  )
  assert.equal(
    getAnalyticsRuntimeConfig('https://preview.scopevcs.com/e/config', enabled),
    null,
  )
})

test('defaults analytics off when runtime configuration is incomplete or invalid', () => {
  assert.equal(getAnalyticsRuntimeConfig('https://scopevcs.com/e/config', {}), null)
  assert.equal(getAnalyticsRuntimeConfig('https://scopevcs.com/e/config', {
    ...enabled,
    SCOPE_ANALYTICS_ENVIRONMENT: 'staging',
  }), null)
  assert.equal(getAnalyticsRuntimeConfig('https://scopevcs.com/e/config', {
    ...enabled,
    SCOPE_ANALYTICS_ORIGIN: 'https://scopevcs.com/app',
  }), null)
})

test('prevents a production destination from being enabled in Railway staging', () => {
  assert.equal(getAnalyticsRuntimeConfig('https://scopevcs.com/e/config', {
    ...enabled,
    RAILWAY_ENVIRONMENT_NAME: 'staging',
  }), null)

  assert.deepEqual(getAnalyticsRuntimeConfig('https://test.scopevcs.com/e/config', {
    ...enabled,
    RAILWAY_ENVIRONMENT_NAME: 'staging',
    SCOPE_ANALYTICS_ENVIRONMENT: 'test',
    SCOPE_ANALYTICS_ORIGIN: 'https://test.scopevcs.com',
  }), {
    environment: 'test',
    release: 'abc123',
    token: 'phc_public',
  })
})

test('prevents a test destination from being enabled in Railway production', () => {
  assert.equal(getAnalyticsRuntimeConfig('https://scopevcs.com/e/config', {
    ...enabled,
    RAILWAY_ENVIRONMENT_NAME: 'production',
    SCOPE_ANALYTICS_ENVIRONMENT: 'test',
  }), null)
})
