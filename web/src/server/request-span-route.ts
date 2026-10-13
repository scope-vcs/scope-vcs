import { trace } from '@opentelemetry/api'
import { analyticsRouteForPathname } from '../analytics/routes'

const SERVER_FUNCTION_ROUTE = '/_serverFn'

export function requestSpanRoute(handlerType: 'serverFn' | 'router', pathname: string) {
  if (handlerType === 'serverFn') return SERVER_FUNCTION_ROUTE
  return analyticsRouteForPathname(pathname)?.path ?? null
}

export function nameActiveRequestSpan(method: string, route: string | null) {
  if (!route) return
  const span = trace.getActiveSpan()
  span?.setAttribute('http.route', route)
  span?.updateName(`${method} ${route}`)
}
