import { analyticsRouteForPathname } from '../analytics/routes'

const SERVER_FUNCTION_ROUTE = '/_serverFn'

export function requestSpanRoute(pathname: string) {
  if (pathname.startsWith(`${SERVER_FUNCTION_ROUTE}/`)) return SERVER_FUNCTION_ROUTE
  return analyticsRouteForPathname(pathname)?.path ?? null
}
