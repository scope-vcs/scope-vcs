import assert from 'node:assert/strict'
import test, { type TestContext } from 'node:test'
import {
  classifyFrontendError,
  reportFrontendError,
} from './diagnostics'
import { createDocumentVitalAttribution, installBrowserDiagnostics } from './browser-diagnostics'

test('buffered frontend reports retain occurrence context through gating and reinstall', (t) => {
  const { browser, installations } = diagnosticBrowser(t)
  t.mock.timers.enable({ apis: ['Date'], now: Date.parse('2026-10-08T18:00:00.000Z') })
  const captures: Array<{ properties: Record<string, number | string | null>; timestamp?: string }> = []
  Object.assign(browser, { location: { pathname: '/private-owner/private-repo/requests/private-id/changes' } })
  reportFrontendError(new Error('private hydration payload'), 'hydration')
  const original = installBrowserDiagnostics({
    release: 'web-original',
    routeName: 'request_changes',
    capture: () => false,
  })
  installations.push(original)
  reportFrontendError(new TypeError('private payload'), 'route')
  original.setRoute('repository_code')
  original.dispose()
  t.mock.timers.tick(60_000)
  const replacement = installBrowserDiagnostics({
    release: 'web-replacement',
    routeName: 'repository_code',
    capture: (_event, properties, timestamp) => { captures.push({ properties, timestamp }); return true },
  })
  installations.push(replacement)
  replacement.flushErrors()

  assert.deepEqual(captures, [{
    properties: {
      error_kind: 'unknown_error',
      error_origin: 'hydration',
      release: 'web-original',
      route_name: 'request_changes',
    },
    timestamp: '2026-10-08T18:00:00.000Z',
  }, {
    properties: {
      error_kind: 'type_error',
      error_origin: 'route',
      release: 'web-original',
      route_name: 'request_changes',
    },
    timestamp: '2026-10-08T18:00:00.000Z',
  }])
})

test('browser diagnostics have one reporting owner across installations and disposal', (t) => {
  const { browser, installations } = diagnosticBrowser(t)
  const captures: Array<Record<string, number | string | null>> = []
  const options = {
    capture: (_event: string, properties: Record<string, number | string | null>) => {
      captures.push(properties)
      return true
    },
    release: 'web-original',
    routeName: 'request_changes',
  }
  const first = installBrowserDiagnostics(options)
  const second = installBrowserDiagnostics(options)
  installations.push(first, second)
  const rejection = () => {
    const event = new Event('unhandledrejection')
    Object.assign(event, { reason: new DOMException('private cancellation', 'AbortError') })
    browser.dispatchEvent(event)
  }

  rejection()
  assert.equal(captures.length, 1)
  assert.equal(captures[0]?.error_kind, 'abort_error')
  second.dispose()
  rejection()
  assert.equal(captures.length, 2)
  first.dispose()
  rejection()
  assert.equal(captures.length, 2)
  const replacement = installBrowserDiagnostics(options)
  installations.push(replacement)
  rejection()
  assert.equal(captures.length, 3)
})

test('frontend errors are reduced to a fixed classification', () => {
  assert.equal(classifyFrontendError(new TypeError('private user text')), 'type_error')
  assert.equal(classifyFrontendError(new Error('https://scope.test/private/path')), 'unknown_error')
  assert.equal(classifyFrontendError('raw rejection value'), 'unknown_error')
  assert.equal(classifyFrontendError(new DOMException('stopped', 'AbortError')), 'abort_error')
})

test('web vitals keep the initial document route and wait for safe identity', () => {
  const captures: Array<{ event: string; properties: Record<string, number | string | null> }> = []
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

function diagnosticBrowser(t: TestContext) {
  const browser = new EventTarget()
  const installations: Array<ReturnType<typeof installBrowserDiagnostics>> = []
  const previousWindow = Object.getOwnPropertyDescriptor(globalThis, 'window')
  Object.defineProperty(globalThis, 'window', { configurable: true, value: browser })
  t.after(() => {
    for (const installation of installations) installation.dispose()
    if (previousWindow) Object.defineProperty(globalThis, 'window', previousWindow)
    else Reflect.deleteProperty(globalThis, 'window')
  })
  return { browser, installations }
}
