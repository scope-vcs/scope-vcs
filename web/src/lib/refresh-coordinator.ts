export type RefreshScheduler = (callback: () => void, delayMs: number) => () => void

export type RefreshCoordinator<Request> = {
  request: (request: Request) => void
  stop: () => void
}

const REFRESH_RETRY_DELAY_MS = 2_000

/**
 * Serialises live refreshes: requests arriving while one is in flight merge
 * into the next, a failed refresh merges its request back and retries after a
 * delay, an optional timeout aborts a hung refresh, and stop cancels all of it.
 * What a request means (reasons, versions) stays with the caller.
 */
export function createRefreshCoordinator<Request>({
  merge,
  refresh,
  schedule,
  shouldRefresh = () => true,
  timeoutMs,
}: {
  merge: (pending: Request, next: Request) => Request
  refresh: (request: Request, signal: AbortSignal) => Promise<unknown>
  schedule: RefreshScheduler
  shouldRefresh?: (request: Request) => boolean
  timeoutMs?: number
}): RefreshCoordinator<Request> {
  let activeController: AbortController | null = null
  let cancelRetry: (() => void) | null = null
  let pending: Request | null = null
  let refreshInFlight = false
  let stopped = false

  const flush = async () => {
    if (stopped || refreshInFlight || pending === null) return
    cancelRetry?.()
    cancelRetry = null
    const request = pending
    pending = null
    if (!shouldRefresh(request)) return
    refreshInFlight = true
    const controller = new AbortController()
    activeController = controller
    const cancelTimeout = timeoutMs === undefined
      ? () => {}
      : schedule(() => controller.abort(), timeoutMs)
    let failed = false
    try {
      await (timeoutMs === undefined
        ? refresh(request, controller.signal)
        : Promise.race([refresh(request, controller.signal), abortRejection(controller.signal)]))
    } catch {
      failed = true
      if (!stopped) pending = pending === null ? request : merge(pending, request)
    } finally {
      cancelTimeout()
      if (activeController === controller) activeController = null
      refreshInFlight = false
    }
    if (stopped || pending === null) return
    if (failed) {
      cancelRetry = schedule(() => {
        cancelRetry = null
        void flush()
      }, REFRESH_RETRY_DELAY_MS)
    } else {
      void flush()
    }
  }

  return {
    request(request) {
      if (stopped) return
      pending = pending === null ? request : merge(pending, request)
      cancelRetry?.()
      cancelRetry = null
      void flush()
    },
    stop() {
      stopped = true
      pending = null
      cancelRetry?.()
      cancelRetry = null
      activeController?.abort()
      activeController = null
    },
  }
}

function abortRejection(signal: AbortSignal) {
  return new Promise<never>((_resolve, reject) => {
    signal.addEventListener(
      'abort',
      () => reject(new Error('Refresh timed out.')),
      { once: true },
    )
  })
}

export function browserScheduler(callback: () => void, delayMs: number) {
  const timeout = window.setTimeout(callback, delayMs)
  return () => window.clearTimeout(timeout)
}
