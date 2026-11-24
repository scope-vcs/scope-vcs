import { useCallback } from 'react'
import { useCachedResource } from '@/lib/use-cached-resource'
import { refreshRequestBacklog, requestBacklogResource, type LoadRequestBacklogPage } from './request-backlog-resource'

export function useRequestBacklog(identity: string | null, version: string, loadPage: LoadRequestBacklogPage) {
  const load = useCallback(
    (signal: AbortSignal) => refreshRequestBacklog(identity ?? '', loadPage, signal),
    [identity, loadPage],
  )
  return useCachedResource({
    identity,
    version,
    load,
    resource: requestBacklogResource,
    fallbackError: 'Could not load these requests.',
  })
}
