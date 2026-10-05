import { STALE_BUILD_HEADER, STALE_BUILD_MESSAGE } from '../lib/stale-build'

const UNKNOWN_SERVER_FUNCTION_PREFIXES = [
  'Server function info not found for ',
  'Invalid server function ID: ',
]

export function staleServerFunctionResponse(error: unknown): Response | null {
  if (!(error instanceof Error)) return null
  if (!UNKNOWN_SERVER_FUNCTION_PREFIXES.some((prefix) => error.message.startsWith(prefix))) return null
  return new Response(STALE_BUILD_MESSAGE, {
    status: 409,
    headers: {
      'cache-control': 'no-store',
      'content-type': 'text/plain; charset=utf-8',
      [STALE_BUILD_HEADER]: '1',
    },
  })
}
