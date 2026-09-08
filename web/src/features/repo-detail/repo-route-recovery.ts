import { HttpError, InvalidApiResponseError } from '../../api/http'
import type { RepoLiveState } from '../../api/types'

export type RepoRouteLoadResult = { live: RepoLiveState } | { unavailable: string }

class RepoRouteUnavailable extends Error {}

export function isRetryableRepoLoadError(error: unknown) {
  if (error instanceof HttpError) return error.response.retryable
  if (error instanceof InvalidApiResponseError) return error.status >= 500
  return error instanceof RepoRouteUnavailable || error instanceof TypeError
}

// Server-function transport failures happen before the API's error envelope
// reaches the browser. Keep their status rather than treating them as access errors.
export const fetchRepoRouteState: typeof fetch = async (input, init) => {
  const response = await fetch(input, init)
  if (response.status >= 500) {
    await response.body?.cancel()
    throw new RepoRouteUnavailable('Repository refresh is temporarily unavailable.')
  }
  return response
}

export async function loadRepoRouteState({
  load,
  refresh,
  signal,
  wait = waitForRetry,
}: {
  load: () => Promise<RepoRouteLoadResult>
  refresh: boolean
  signal: AbortSignal
  wait?: (signal: AbortSignal) => Promise<void>
}): Promise<RepoLiveState> {
  for (;;) {
    signal.throwIfAborted()
    try {
      const result = await load()
      if ('live' in result) return result.live
      throw new RepoRouteUnavailable(result.unavailable)
    } catch (error) {
      if (!refresh || !isRetryableRepoLoadError(error)) throw error
      signal.throwIfAborted()
      // The router owns the current data and this attempt. Keeping a background
      // reload pending leaves the stream owner mounted until recovery or navigation.
      await wait(signal)
    }
  }
}

function waitForRetry(signal: AbortSignal) {
  return new Promise<void>((resolve, reject) => {
    const finish = () => {
      signal.removeEventListener('abort', abort)
      resolve()
    }
    const timer = setTimeout(finish, 2_000)
    const abort = () => {
      clearTimeout(timer)
      reject(signal.reason)
    }
    signal.addEventListener('abort', abort, { once: true })
    if (signal.aborted) abort()
  })
}
