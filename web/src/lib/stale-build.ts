import { toast } from 'sonner'
import { SIGN_IN_REQUIRED_HEADER, SignInRequiredError } from '../api/sign-in-required'
import { withSessionRetry } from './session-retry-fetch'
import { sessionReady, whenSessionReady } from './viewer-state'

export const STALE_BUILD_HEADER = 'x-scope-stale-build'

export const STALE_BUILD_MESSAGE = 'Scope was updated. Reload to continue.'

export class StaleBuildError extends Error {
  constructor() {
    super(STALE_BUILD_MESSAGE)
    this.name = 'StaleBuildError'
  }
}

let staleBuild = false

const sendServerFunction = withSessionRetry(
  (input, init) => fetch(input, init),
  { ready: sessionReady, whenReady: whenSessionReady },
)

export const fetchServerFunction: typeof fetch = async (input, init) => {
  if (staleBuild) throw new StaleBuildError()
  const response = await sendServerFunction(input, init)
  if (response.headers.has(SIGN_IN_REQUIRED_HEADER)) {
    await response.body?.cancel()
    throw new SignInRequiredError()
  }
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
