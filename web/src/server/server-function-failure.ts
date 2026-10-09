import { SIGN_IN_REQUIRED_HEADER, SignInRequiredError } from '../api/sign-in-required'
import { STALE_BUILD_HEADER, STALE_BUILD_MESSAGE } from '../lib/stale-build'

const UNKNOWN_SERVER_FUNCTION_PREFIXES = [
  'Server function info not found for ',
  'Invalid server function ID: ',
]

export function serverFunctionFailureResponse(error: unknown): Response | null {
  if (error instanceof SignInRequiredError) return plainTextResponse(401, error.message, SIGN_IN_REQUIRED_HEADER)
  if (!(error instanceof Error)) return null
  if (!UNKNOWN_SERVER_FUNCTION_PREFIXES.some((prefix) => error.message.startsWith(prefix))) return null
  return plainTextResponse(409, STALE_BUILD_MESSAGE, STALE_BUILD_HEADER)
}

function plainTextResponse(status: number, message: string, marker: string) {
  return new Response(message, {
    status,
    headers: {
      'cache-control': 'no-store',
      'content-type': 'text/plain; charset=utf-8',
      [marker]: '1',
    },
  })
}
