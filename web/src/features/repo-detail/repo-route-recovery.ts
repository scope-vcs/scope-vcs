import { HttpError, InvalidApiResponseError } from '../../api/http'
import type { RepoLiveState } from '../../api/types'
import { fetchServerFunction, StaleBuildError } from '../../lib/stale-build'

export type RepoRouteState = RepoLiveState & { refreshId: string }

export type RepoRouteLoadResult = { live: RepoLiveState } | { unavailable: string }

class RepoRouteUnavailable extends Error {}

export function isRetryableRepoLoadError(error: unknown) {
  if (error instanceof HttpError) return error.response.retryable
  if (error instanceof InvalidApiResponseError) return error.status >= 500
  return error instanceof RepoRouteUnavailable || error instanceof TypeError
}

export const fetchRepoRouteState: typeof fetch = async (input, init) => {
  const response = await fetchServerFunction(input, init)
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
}): Promise<RepoRouteState> {
  for (;;) {
    signal.throwIfAborted()
    try {
      const result = await load()
      if ('live' in result) {
        signal.throwIfAborted()
        return { ...result.live, refreshId: crypto.randomUUID() }
      }
      throw new RepoRouteUnavailable(result.unavailable)
    } catch (error) {
      if (!refresh) throw error
      if (error instanceof StaleBuildError) return await waitForAbort(signal)
      if (!isRetryableRepoLoadError(error)) throw error
      signal.throwIfAborted()
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

function waitForAbort(signal: AbortSignal) {
  return new Promise<never>((_resolve, reject) => {
    const abort = () => reject(signal.reason)
    signal.addEventListener('abort', abort, { once: true })
    if (signal.aborted) abort()
  })
}
