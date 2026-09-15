import type { AccountSessionResponse } from '../../api/types.generated'
import { createCachedResource } from '../../lib/cached-resource'

type AccountSessionResourceValue = {
  account: AccountSessionResponse | null
}

type AccountSessionLoader = (
  signal: AbortSignal,
) => Promise<AccountSessionResponse | null>

const retryDelays = [200, 800] as const
let activeViewerId: string | null = null

const accountSessionResource =
  createCachedResource<AccountSessionResourceValue>({ maxEntries: 1 })

export function activateAccountSessionViewer(viewerId: string) {
  if (activeViewerId !== null && activeViewerId !== viewerId) {
    accountSessionResource.clear()
  }
  activeViewerId = viewerId
}

export async function loadAccountSessionForViewer(
  viewerId: string,
  load: AccountSessionLoader,
  options: {
    retryDelays?: readonly number[]
    wait?: (delay: number, signal: AbortSignal) => Promise<void>
  } = {},
) {
  activateAccountSessionViewer(viewerId)
  const value = await accountSessionResource.load(
    resourceKey(viewerId),
    '',
    (signal) => loadWithRetry(
      load,
      signal,
      options.retryDelays ?? retryDelays,
      options.wait ?? abortableDelay,
    ),
  )
  return value.account
}

export function resetAccountSessionResource() {
  activeViewerId = null
  accountSessionResource.clear()
}

async function loadWithRetry(
  load: AccountSessionLoader,
  signal: AbortSignal,
  delays: readonly number[],
  wait: (delay: number, signal: AbortSignal) => Promise<void>,
) {
  for (let attempt = 0; ; attempt += 1) {
    try {
      return { account: await load(signal) }
    } catch (error) {
      if (signal.aborted || attempt >= delays.length) throw error
      await wait(delays[attempt], signal)
    }
  }
}

function resourceKey(viewerId: string) {
  return `account-session\0${viewerId}`
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
