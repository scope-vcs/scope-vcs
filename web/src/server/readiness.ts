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
  let apiReady = false
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
    apiReady = response.ok
  } catch {
    // Health responses must not expose connection details or upstream errors.
  }

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
