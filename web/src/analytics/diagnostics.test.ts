import assert from 'node:assert/strict'
import test from 'node:test'
import {
  classifyFrontendError,
  createDocumentVitalAttribution,
} from './diagnostics'

test('frontend errors are reduced to a fixed classification', () => {
  assert.equal(classifyFrontendError(new TypeError('private user text')), 'type_error')
  assert.equal(classifyFrontendError(new Error('https://scope.test/private/path')), 'unknown_error')
  assert.equal(classifyFrontendError('raw rejection value'), 'unknown_error')
  assert.equal(classifyFrontendError(new DOMException('stopped', 'AbortError')), 'abort_error')
})

test('web vitals keep the initial document route and wait for safe identity', () => {
  const captures: Array<{ event: string; properties: Record<string, number | string> }> = []
  let ready = false
  const measurements = createDocumentVitalAttribution((event, properties) => {
    if (!ready) return false
    captures.push({ event, properties })
    return true
  })

  measurements.activate('request_changes')
  measurements.report('LCP', 510)
  measurements.report('CLS', 0.03)
  measurements.report('INP', 60)
  measurements.activate('request_changes')
  assert.deepEqual(captures, [])

  ready = true
  measurements.flush()

  assert.deepEqual(captures, [
    {
      event: 'web_vital',
      properties: { metric: 'LCP', route_name: 'request_changes', value: 510 },
    },
    {
      event: 'web_vital',
      properties: { metric: 'CLS', route_name: 'request_changes', value: 0.03 },
    },
    {
      event: 'web_vital',
      properties: { metric: 'INP', route_name: 'request_changes', value: 60 },
    },
  ])
})
