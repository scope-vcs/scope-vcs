import type { AnalyticsProxyOptions } from './analytics-proxy'
import { proxyAnalyticsCapture } from './analytics-proxy'
import { getAnalyticsRuntimeConfig } from './analytics-runtime-config'

type AnalyticsEndpointOptions = AnalyticsProxyOptions & {
  runtime?: Readonly<Record<string, string | undefined>>
}

export async function analyticsEndpointResponse(
  request: Request,
  options: AnalyticsEndpointOptions = {},
) {
  const url = new URL(request.url)
  if (url.pathname !== '/e' && !url.pathname.startsWith('/e/')) return null

  const config = getAnalyticsRuntimeConfig(url, options.runtime)
  if (url.pathname === '/e/config') {
    if (request.method !== 'GET') return methodNotAllowed('GET')
    return Response.json(config, {
      headers: { 'cache-control': 'no-store' },
    })
  }

  if (url.pathname !== '/e/e/') return endpointResponse('Not found.', 404)
  if (request.method !== 'POST') return methodNotAllowed('POST')
  if (!config) return endpointResponse('Not found.', 404)
  return proxyAnalyticsCapture(request, options)
}

function methodNotAllowed(method: string) {
  const response = endpointResponse('Method not allowed.', 405)
  response.headers.set('allow', method)
  return response
}

function endpointResponse(message: string, status: number) {
  return new Response(message, {
    headers: { 'cache-control': 'no-store' },
    status,
  })
}
