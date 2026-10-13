import assert from 'node:assert/strict'
import test from 'node:test'
import { getTelemetryConfig } from './telemetry-config'

test('stays off without a project token', () => {
  assert.deepEqual(getTelemetryConfig({}), { enabled: false, reason: 'unconfigured' })
  assert.deepEqual(
    getTelemetryConfig({ POSTHOG_PROJECT_TOKEN: '   ' }),
    { enabled: false, reason: 'unconfigured' },
  )
})

test('exports to PostHog US cloud by default with the bearer token', () => {
  assert.deepEqual(getTelemetryConfig({ POSTHOG_PROJECT_TOKEN: ' phc_public ' }), {
    enabled: true,
    headers: { Authorization: 'Bearer phc_public' },
    logsUrl: 'https://us.i.posthog.com/i/v1/logs',
    resourceAttributes: { 'service.name': 'web' },
    tracesUrl: 'https://us.i.posthog.com/i/v1/traces',
  })
})

test('uses the configured host and describes the deployment', () => {
  assert.deepEqual(getTelemetryConfig({
    POSTHOG_HOST: ' https://eu.i.posthog.com/ ',
    POSTHOG_PROJECT_TOKEN: 'phc_public',
    RAILWAY_ENVIRONMENT_NAME: 'production',
    SCOPE_ANALYTICS_RELEASE: ' abc123 ',
  }), {
    enabled: true,
    headers: { Authorization: 'Bearer phc_public' },
    logsUrl: 'https://eu.i.posthog.com/i/v1/logs',
    resourceAttributes: {
      'deployment.environment.name': 'production',
      'service.name': 'web',
      'service.version': 'abc123',
    },
    tracesUrl: 'https://eu.i.posthog.com/i/v1/traces',
  })
})

test('stays off when injected OTLP variables would redirect the export', () => {
  for (const name of [
    'OTEL_EXPORTER_OTLP_ENDPOINT',
    'OTEL_EXPORTER_OTLP_HEADERS',
    'OTEL_EXPORTER_OTLP_TRACES_ENDPOINT',
    'OTEL_EXPORTER_OTLP_TRACES_HEADERS',
    'OTEL_EXPORTER_OTLP_LOGS_ENDPOINT',
    'OTEL_EXPORTER_OTLP_LOGS_HEADERS',
  ]) {
    assert.deepEqual(
      getTelemetryConfig({ POSTHOG_PROJECT_TOKEN: 'phc_public', [name]: 'https://railway.example' }),
      { enabled: false, reason: 'injected-otlp-variables' },
      name,
    )
  }
})

test('ignores blank OTLP variables and unrelated OTLP settings', () => {
  const config = getTelemetryConfig({
    OTEL_EXPORTER_OTLP_ENDPOINT: ' ',
    OTEL_EXPORTER_OTLP_PROTOCOL: 'http/protobuf',
    POSTHOG_PROJECT_TOKEN: 'phc_public',
  })
  assert.equal(config.enabled, true)
})
