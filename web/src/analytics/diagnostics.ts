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

type FrontendErrorReport = {
  kind: FrontendErrorKind
  origin: FrontendErrorOrigin
  occurredAt: string
  routeName: string | null
  release: string | null | undefined
}

export type DiagnosticCapture = (
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

export function installFrontendDiagnostics({
  capture,
  release,
  routeName,
}: DiagnosticInstallation) {
  const installation = { capture, release, routeName }
  if (documentRelease === undefined) documentRelease = release
  const firstInstallation = installations.size === 0
  installations.add(installation)
  flushPendingErrors()

  return {
    firstInstallation,
    dispose() {
      return installations.delete(installation) && installations.size === 0
    },
    flushErrors: flushPendingErrors,
    setRoute(nextRouteName: string | null) {
      installation.routeName = nextRouteName
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

function retainPendingError(report: FrontendErrorReport) {
  if (pendingErrors.length === 10) pendingErrors.shift()
  pendingErrors.push(report)
}

export function captureWebVital(properties: Record<string, number | string | null>) {
  const installation = currentInstallation()
  return installation ? installation.capture('web_vital', properties) : false
}
