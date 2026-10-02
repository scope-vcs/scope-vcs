import { STALE_BUILD_HEADER, STALE_BUILD_MESSAGE } from '../lib/stale-build'

// TanStack Start's resolver is internal, so its error text is the only signal.
// The built-server smoke requests an unknown ID, so an upgrade that changes the text fails there.
const UNKNOWN_SERVER_FUNCTION = 'Server function info not found for '

/**
 * A tab built before a deploy can call a server function the current build no
 * longer has. Answer with a plain-text 409 rather than a generic JSON 500: the
 * header tells current clients to stop, and older clients throw on a non-JSON
 * error instead of treating the body as a result.
 */
export function staleServerFunctionResponse(error: unknown): Response | null {
  if (!(error instanceof Error) || !error.message.startsWith(UNKNOWN_SERVER_FUNCTION)) return null
  return new Response(STALE_BUILD_MESSAGE, {
    status: 409,
    headers: {
      'cache-control': 'no-store',
      'content-type': 'text/plain; charset=utf-8',
      [STALE_BUILD_HEADER]: '1',
    },
  })
}
