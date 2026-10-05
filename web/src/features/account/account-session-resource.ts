import type { AccountSessionResponse } from '../../api/types.generated'
import { createCachedResource } from '../../lib/cached-resource'

export type AccountSessionResourceValue = {
  account: AccountSessionResponse | null
}

export type AccountSessionLoader = (
  signal: AbortSignal,
) => Promise<AccountSessionResponse | null>

const retryDelays = [200, 800] as const
export const accountSessionResource =
  createCachedResource<AccountSessionResourceValue>({ maxEntries: 1 })

export function accountSessionIdentity(viewerId: string) {
  return `account-session\0${viewerId}`
}

export async function loadAccountSessionValue(
  load: AccountSessionLoader,
  signal: AbortSignal,
  options: {
    retryDelays?: readonly number[]
    wait?: (delay: number, signal: AbortSignal) => Promise<void>
  } = {},
): Promise<AccountSessionResourceValue> {
  const delays = options.retryDelays ?? retryDelays
  const wait = options.wait ?? abortableDelay
  for (let attempt = 0; ; attempt += 1) {
    try {
      return { account: await load(signal) }
    } catch (error) {
      if (signal.aborted || attempt >= delays.length) throw error
      await wait(delays[attempt], signal)
    }
  }
}

function abortableDelay(delay: number, signal: AbortSignal) {
  return new Promise<void>((resolve, reject) => {
    const onAbort = () => {
      window.clearTimeout(timeout)
      reject(signal.reason)
    }
    const timeout = window.setTimeout(() => {
      signal.removeEventListener('abort', onAbort)
      resolve()
    }, delay)
    if (signal.aborted) {
      onAbort()
    } else {
      signal.addEventListener('abort', onAbort, { once: true })
    }
  })
}
