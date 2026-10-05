import { toast } from 'sonner'

export const STALE_BUILD_HEADER = 'x-scope-stale-build'

export const STALE_BUILD_MESSAGE = 'Scope was updated. Reload to continue.'

export class StaleBuildError extends Error {
  constructor() {
    super(STALE_BUILD_MESSAGE)
    this.name = 'StaleBuildError'
  }
}

let staleBuild = false

export const fetchServerFunction: typeof fetch = async (input, init) => {
  if (staleBuild) throw new StaleBuildError()
  const response = await fetch(input, init)
  if (!response.headers.has(STALE_BUILD_HEADER)) return response
  await response.body?.cancel()
  staleBuild = true
  toast(STALE_BUILD_MESSAGE, {
    id: STALE_BUILD_HEADER,
    duration: Number.POSITIVE_INFINITY,
    action: { label: 'Reload', onClick: () => window.location.reload() },
  })
  throw new StaleBuildError()
}
