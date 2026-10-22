import type { AnalyticsProxyOptions } from './analytics-proxy'
import { proxyAnalyticsCapture } from './analytics-proxy'
import { getAnalyticsRuntimeConfig } from './analytics-runtime-config'

export async function analyticsEndpointResponse(
  request: Request,
  options: AnalyticsProxyOptions = {},
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
  return proxyAnalyticsCapture(request, config, options)
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
