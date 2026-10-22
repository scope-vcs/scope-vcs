import { context, ROOT_CONTEXT, SpanKind, TraceFlags, trace } from '@opentelemetry/api'
import { SamplingDecision } from '@opentelemetry/sdk-trace'
import assert from 'node:assert/strict'
import test from 'node:test'
import { requestTraceSampler } from './telemetry-sampler'

const traceId = '00000000000000000000000000000001'

function decision(parentFlags: TraceFlags | null, kind: SpanKind) {
  const parent = parentFlags === null
    ? ROOT_CONTEXT
    : trace.setSpanContext(context.active(), {
      isRemote: true,
      spanId: '0000000000000002',
      traceFlags: parentFlags,
      traceId,
    })
  return requestTraceSampler().shouldSample(parent, traceId, 'span', kind, {}, []).decision
}

test('starts traces only at incoming server requests', () => {
  assert.equal(decision(null, SpanKind.SERVER), SamplingDecision.RECORD_AND_SAMPLED)
  assert.equal(decision(null, SpanKind.CLIENT), SamplingDecision.NOT_RECORD)
  assert.equal(decision(null, SpanKind.INTERNAL), SamplingDecision.NOT_RECORD)
})

test('follows the sampling decision of an existing parent', () => {
  assert.equal(decision(TraceFlags.SAMPLED, SpanKind.CLIENT), SamplingDecision.RECORD_AND_SAMPLED)
  assert.equal(decision(TraceFlags.NONE, SpanKind.SERVER), SamplingDecision.NOT_RECORD)
})
