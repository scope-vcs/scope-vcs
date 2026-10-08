import type { Metric } from 'web-vitals'
import { analyticsRouteForPathname } from './routes'

type FrontendErrorKind =
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

type FrontendErrorOrigin =
  | 'hydration'
  | 'promise'
  | 'route'
  | 'window'

type WebVitalMetric = 'CLS' | 'INP' | 'LCP'

type FrontendErrorReport = {
  kind: FrontendErrorKind
  origin: FrontendErrorOrigin
  occurredAt: string
  routeName: string | null
  release: string | null | undefined
}

type DiagnosticCapture = (
  event: 'frontend_error' | 'web_vital',
  properties: Record<string, number | string | null>,
  occurredAt?: string,
) => boolean | void

type DiagnosticInstallation = {
  capture: DiagnosticCapture
  routeName: string | null
  release: string | null
}

const installations = new Set<DiagnosticInstallation>()
let documentRelease: string | null | undefined
const pendingErrors: FrontendErrorReport[] = []
const documentVitals = createDocumentVitalAttribution((event, properties) => {
  const installation = currentInstallation()
  return installation ? installation.capture(event, properties) : false
})
let webVitalsStarted = false

export function reportFrontendError(
  error: unknown,
  origin: FrontendErrorOrigin,
) {
  const installation = currentInstallation()
  const report: FrontendErrorReport = {
    kind: classifyFrontendError(error),
    origin,
    occurredAt: new Date().toISOString(),
    routeName: installation ? installation.routeName : documentRouteName(),
    release: installation ? installation.release : documentRelease,
  }
  if (!installation || !captureError(installation, report)) retainPendingError(report)
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
  release,
  routeName,
}: DiagnosticInstallation) {
  documentVitals.activate(routeName)
  startWebVitals()

  const installation = { capture, release, routeName }
  if (documentRelease === undefined) documentRelease = release
  if (installations.size === 0) {
    window.addEventListener('error', onWindowError)
    window.addEventListener('unhandledrejection', onUnhandledRejection)
  }
  installations.add(installation)
  flushPendingErrors()

  return {
    dispose() {
      if (!installations.delete(installation)) return
      if (installations.size === 0) {
        window.removeEventListener('error', onWindowError)
        window.removeEventListener('unhandledrejection', onUnhandledRejection)
      }
    },
    flushVitals() {
      documentVitals.flush()
    },
    flushErrors: flushPendingErrors,
    setRoute(nextRouteName: string | null) {
      installation.routeName = nextRouteName
      documentVitals.activate(nextRouteName)
    },
  }
}

function documentRouteName() {
  if (typeof window === 'undefined') return null
  return analyticsRouteForPathname(window.location.pathname)?.name ?? null
}

function currentInstallation() {
  return Array.from(installations).at(-1)
}

function captureError(installation: DiagnosticInstallation, report: FrontendErrorReport) {
  if (!report.routeName) return true
  return installation.capture('frontend_error', {
    error_kind: report.kind,
    error_origin: report.origin,
    release: report.release === undefined ? documentRelease ?? null : report.release,
    route_name: report.routeName,
  }, report.occurredAt) !== false
}

function flushPendingErrors() {
  const installation = currentInstallation()
  if (!installation) return
  for (const report of pendingErrors.splice(0)) {
    if (!captureError(installation, report)) retainPendingError(report)
  }
}

function onWindowError(event: ErrorEvent) {
  reportFrontendError(event.error, 'window')
}

function onUnhandledRejection(event: PromiseRejectionEvent) {
  reportFrontendError(event.reason, 'promise')
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
  }).catch(() => {})
}

function reportWebVital(metric: Metric) {
  if (metric.name === 'CLS' || metric.name === 'INP' || metric.name === 'LCP') {
    documentVitals.report(metric.name, metric.value)
  }
}
