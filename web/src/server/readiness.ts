import { getApiConnection } from '../api/client'

const apiReadinessTimeoutMs = 3_000

type Fetch = (
  input: RequestInfo | URL,
  init?: RequestInit,
) => Promise<Response>

export async function readinessResponse(
  fetchApi: Fetch = globalThis.fetch,
  timeoutMs = apiReadinessTimeoutMs,
) {
  const apiReady = await apiIsReady(fetchApi, timeoutMs)

  return Response.json(
    {
      status: apiReady ? 'ok' : 'unavailable',
      service: 'web',
      checks: [{
        name: 'api',
        status: apiReady ? 'ok' : 'unavailable',
      }],
    },
    {
      headers: { 'cache-control': 'no-store' },
      status: apiReady ? 200 : 503,
    },
  )
}

async function apiIsReady(fetchApi: Fetch, timeoutMs: number) {
  try {
    const response = await fetchApi(
      `${getApiConnection('checking API readiness')}/readyz`,
      {
        headers: { accept: 'application/json' },
        method: 'GET',
        redirect: 'error',
        signal: AbortSignal.timeout(timeoutMs),
      },
    )
    await response.body?.cancel()
    return response.ok
  } catch {
    return false
  }
}
