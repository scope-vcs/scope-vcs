import type { Metric } from 'web-vitals'

export type FrontendErrorKind =
  | 'abort_error'
  | 'aggregate_error'
  | 'dom_error'
  | 'eval_error'
  | 'range_error'
  | 'reference_error'
  | 'syntax_error'
  | 'type_error'
  | 'unknown_error'
  | 'uri_error'

export type FrontendErrorOrigin =
  | 'hydration'
  | 'promise'
  | 'route'
  | 'window'

export type WebVitalMetric = 'CLS' | 'INP' | 'LCP'

type FrontendErrorReport = {
  kind: FrontendErrorKind
  origin: FrontendErrorOrigin
}

type DiagnosticCapture = (
  event: 'frontend_error' | 'web_vital',
  properties: Record<string, number | string>,
) => boolean | void

const errorSubscribers = new Set<(
  report: FrontendErrorReport,
) => boolean | void>()
const pendingErrors: FrontendErrorReport[] = []
const vitalSubscribers = new Set<DiagnosticCapture>()
const documentVitals = createDocumentVitalAttribution((event, properties) => {
  let accepted = false
  for (const subscriber of vitalSubscribers) {
    accepted = subscriber(event, properties) !== false || accepted
  }
  return accepted
})
let webVitalsStarted = false

export function reportFrontendError(
  error: unknown,
  origin: FrontendErrorOrigin,
) {
  const report = { kind: classifyFrontendError(error), origin }
  if (errorSubscribers.size === 0) {
    retainPendingError(report)
    return
  }
  let accepted = false
  for (const subscriber of errorSubscribers) {
    accepted = subscriber(report) !== false || accepted
  }
  if (!accepted) retainPendingError(report)
}

export function classifyFrontendError(error: unknown): FrontendErrorKind {
  if (error instanceof AggregateError) return 'aggregate_error'
  if (error instanceof EvalError) return 'eval_error'
  if (error instanceof RangeError) return 'range_error'
  if (error instanceof ReferenceError) return 'reference_error'
  if (error instanceof SyntaxError) return 'syntax_error'
  if (error instanceof TypeError) return 'type_error'
  if (error instanceof URIError) return 'uri_error'
  if (error instanceof DOMException) {
    return error.name === 'AbortError' ? 'abort_error' : 'dom_error'
  }
  return 'unknown_error'
}

export function createDocumentVitalAttribution(capture: DiagnosticCapture) {
  let initialRouteName: string | null | undefined
  const pending = new Map<WebVitalMetric, number>()

  return {
    activate(routeName: string | null) {
      if (initialRouteName === undefined) initialRouteName = routeName
    },
    flush() {
      if (!initialRouteName) return
      for (const [metric, value] of pending) {
        if (capture('web_vital', {
          metric,
          route_name: initialRouteName,
          value,
        }) !== false) {
          pending.delete(metric)
        }
      }
    },
    report(metric: WebVitalMetric, value: number) {
      if (!Number.isFinite(value) || value < 0) return
      pending.set(metric, value)
      this.flush()
    },
  }
}

export function installBrowserDiagnostics({
  capture,
  routeName,
}: {
  capture: DiagnosticCapture
  routeName: string | null
}) {
  documentVitals.activate(routeName)
  vitalSubscribers.add(capture)
  startWebVitals()

  const captureError = ({ kind, origin }: FrontendErrorReport) => {
    if (!routeName) return false
    return capture('frontend_error', {
      error_kind: kind,
      error_origin: origin,
      route_name: routeName,
    })
  }
  errorSubscribers.add(captureError)
  flushPendingErrors()

  const onWindowError = (event: ErrorEvent) => {
    reportFrontendError(event.error, 'window')
  }
  const onUnhandledRejection = (event: PromiseRejectionEvent) => {
    reportFrontendError(event.reason, 'promise')
  }

  window.addEventListener('error', onWindowError)
  window.addEventListener('unhandledrejection', onUnhandledRejection)

  return {
    dispose() {
      errorSubscribers.delete(captureError)
      vitalSubscribers.delete(capture)
      window.removeEventListener('error', onWindowError)
      window.removeEventListener('unhandledrejection', onUnhandledRejection)
    },
    flushVitals() {
      documentVitals.flush()
    },
    flushErrors: flushPendingErrors,
    setRoute(nextRouteName: string | null) {
      routeName = nextRouteName
      documentVitals.activate(nextRouteName)
    },
  }

  function flushPendingErrors() {
    for (const report of pendingErrors.splice(0)) {
      if (captureError(report) === false) retainPendingError(report)
    }
  }
}

function retainPendingError(report: FrontendErrorReport) {
  if (pendingErrors.length === 10) pendingErrors.shift()
  pendingErrors.push(report)
}

function startWebVitals() {
  if (webVitalsStarted) return
  webVitalsStarted = true
  void import('web-vitals').then(({ onCLS, onINP, onLCP }) => {
    onCLS(reportWebVital)
    onINP(reportWebVital)
    onLCP(reportWebVital)
  }).catch(() => {
    // Diagnostics are best effort and cannot affect the application.
  })
}

function reportWebVital(metric: Metric) {
  if (metric.name === 'CLS' || metric.name === 'INP' || metric.name === 'LCP') {
    documentVitals.report(metric.name, metric.value)
  }
}
