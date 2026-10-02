import { toast } from 'sonner'

/** Set by the server when a server-function call names a function this build no longer has. */
export const STALE_BUILD_HEADER = 'x-scope-stale-build'

export const STALE_BUILD_MESSAGE = 'Scope was updated. Reload to continue.'

/** The tab is running an older build than the server. Retrying cannot succeed. */
export class StaleBuildError extends Error {
  constructor() {
    super(STALE_BUILD_MESSAGE)
    this.name = 'StaleBuildError'
  }
}

/**
 * Server-function transport. A stale-build response becomes a StaleBuildError
 * and a persistent reload notice instead of a result the caller would retry.
 */
export const fetchServerFunction: typeof fetch = async (input, init) => {
  const response = await fetch(input, init)
  if (!response.headers.has(STALE_BUILD_HEADER)) return response
  await response.body?.cancel()
  toast(STALE_BUILD_MESSAGE, {
    id: STALE_BUILD_HEADER,
    duration: Number.POSITIVE_INFINITY,
    action: { label: 'Reload', onClick: () => window.location.reload() },
  })
  throw new StaleBuildError()
}
