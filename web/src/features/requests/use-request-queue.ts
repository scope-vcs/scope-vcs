import { useCallback, useEffect } from 'react'
import { useCachedResource } from '@/lib/use-cached-resource'
import { nextRequestAttentionAt } from './request-list-model'
import { refreshRequestQueue, requestQueueResource, type LoadRequestQueuePage } from './request-queue-cache'

export function useRequestQueue(identity: string | null, version: string, loadPage: LoadRequestQueuePage) {
  const load = useCallback((signal: AbortSignal) => refreshRequestQueue(identity!, loadPage, signal), [identity, loadPage])
  const current = useCachedResource({ identity, version, load, resource: requestQueueResource, fallbackError: 'Could not load requests.' })
  const expiry = current.value ? nextRequestAttentionAt(current.value.pages) : null
  useEffect(() => {
    if (!identity || expiry === null) return
    const delay = Math.max(0, expiry * 1000 - Date.now()) + 100
    const timer = window.setTimeout(() => requestQueueResource.invalidate(identity), Math.min(delay, 2_147_483_647))
    return () => window.clearTimeout(timer)
  }, [expiry, identity])
  return current
}
