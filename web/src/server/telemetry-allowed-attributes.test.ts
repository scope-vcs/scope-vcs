import { SpanKind, SpanStatusCode, TraceFlags } from '@opentelemetry/api'
import { resourceFromAttributes } from '@opentelemetry/resources'
import type { ReadableSpan, SpanExporter } from '@opentelemetry/sdk-trace-node'
import assert from 'node:assert/strict'
import test, { mock } from 'node:test'
import { AllowedAttributesExporter } from './telemetry-allowed-attributes'

function span(overrides: Partial<ReadableSpan>): ReadableSpan {
  const spanContext = {
    spanId: '0000000000000001',
    traceFlags: TraceFlags.SAMPLED,
    traceId: '00000000000000000000000000000001',
  }
  return {
    attributes: {},
    droppedAttributesCount: 0,
    droppedEventsCount: 0,
    droppedLinksCount: 0,
    duration: [0, 10],
    ended: true,
    endTime: [1, 10],
    events: [],
    instrumentationScope: { name: 'scope-test' },
    kind: SpanKind.SERVER,
    links: [],
    name: 'GET',
    resource: resourceFromAttributes({ 'service.name': 'web' }),
    spanContext: () => spanContext,
    startTime: [1, 0],
    status: { code: SpanStatusCode.UNSET },
    ...overrides,
  }
}

function exported(input: ReadableSpan) {
  const downstream = mock.fn<SpanExporter['export']>((_spans, done) => done({ code: 0 }))
  const resultCallback = mock.fn()
  new AllowedAttributesExporter({
    export: downstream,
    shutdown: async () => {},
  }).export([input], resultCallback)
  assert.equal(downstream.mock.callCount(), 1)
  assert.equal(resultCallback.mock.callCount(), 1)
  return downstream.mock.calls[0].arguments[0][0]
}

test('exports only reviewed request attributes', () => {
  const result = exported(span({
    attributes: {
      'http.request.method': 'GET',
      'http.response.status_code': 200,
      'http.route': '/$owner/$repo',
      'network.protocol.version': '1.1',
      'server.address': 'api.internal',
      'server.port': 8080,
      'url.full': 'https://scopevcs.com/acme/private-app?token=secret',
      'url.path': '/acme/private-app',
      'url.query': 'token=secret',
      'url.scheme': 'https',
      'user_agent.original': 'curl/8',
    },
  }))
  assert.deepEqual(result.attributes, {
    'http.request.method': 'GET',
    'http.response.status_code': 200,
    'http.route': '/$owner/$repo',
    'network.protocol.version': '1.1',
    'server.address': 'api.internal',
    'server.port': 8080,
    'url.scheme': 'https',
  })
})

test('drops events and error messages that can carry repository content', () => {
  const result = exported(span({
    events: [{
      attributes: { path: 'README.md' },
      name: 'loaded acme/private-app/README.md',
      time: [1, 5],
    }],
    status: { code: SpanStatusCode.ERROR, message: 'repository acme/private-app failed' },
  }))
  assert.deepEqual(result.events, [])
  assert.deepEqual(result.status, { code: SpanStatusCode.ERROR, message: '' })
})

test('keeps span identity and successful status', () => {
  const input = span({ status: { code: SpanStatusCode.OK } })
  const result = exported(input)
  assert.deepEqual(result.spanContext(), input.spanContext())
  assert.deepEqual(result.status, { code: SpanStatusCode.OK })
})
