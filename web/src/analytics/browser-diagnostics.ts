import type { Metric } from 'web-vitals'
import {
  captureWebVital,
  installFrontendDiagnostics,
  reportFrontendError,
  type DiagnosticCapture,
} from './diagnostics'

type WebVitalMetric = 'CLS' | 'INP' | 'LCP'

const documentVitals = createDocumentVitalAttribution((_event, properties) => captureWebVital(properties))
let webVitalsStarted = false

export function installBrowserDiagnostics(options: Parameters<typeof installFrontendDiagnostics>[0]) {
  const session = installFrontendDiagnostics(options)
  documentVitals.activate(options.routeName)
  startWebVitals()
  if (session.firstInstallation) {
    window.addEventListener('error', onWindowError)
    window.addEventListener('unhandledrejection', onUnhandledRejection)
  }

  return {
    dispose() {
      if (!session.dispose()) return
      window.removeEventListener('error', onWindowError)
      window.removeEventListener('unhandledrejection', onUnhandledRejection)
    },
    flushErrors: session.flushErrors,
    flushVitals() {
      documentVitals.flush()
    },
    setRoute(routeName: string | null) {
      session.setRoute(routeName)
      documentVitals.activate(routeName)
    },
  }
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

function onWindowError(event: ErrorEvent) {
  reportFrontendError(event.error, 'window')
}

function onUnhandledRejection(event: PromiseRejectionEvent) {
  reportFrontendError(event.reason, 'promise')
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
