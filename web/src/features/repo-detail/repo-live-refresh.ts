import type { RepoLiveState } from '@/api/types'
import type { RepoChangeEvent } from '@/api/types.generated'
import { useAuth } from '@clerk/tanstack-react-start'
import { useCallback, useEffect, useRef } from 'react'
import { runRepoEventStream, streamRepoEvents } from './repo-event-stream'
import { repoResourceScope } from './repo-resource-scope'
import { invalidateRepoResources, resyncTopics, topicsOf } from './repo-resource'
import { repoSummaryChangedBy, repoSummaryResource } from './repo-summary-resource'

type RepoChangeListener = (event: RepoChangeEvent) => void
export type SubscribeToRepoChanges = (
  listener: RepoChangeListener,
) => () => void

export function useRepoLiveRefresh(live: RepoLiveState | null, summaryIdentity: string) {
  const { getToken, isLoaded, userId } = useAuth()
  const scope = live && isLoaded ? repoResourceScope(live.repo, userId ?? null) : null
  const repoId = live?.repo.id
  const versioned = live?.repo.access.actor !== 'Public'
  const eventStreamUrl = live?.event_stream_url
  const tokenTemplate = live?.clerk_token_template
  const listenersRef = useRef(new Set<RepoChangeListener>())
  const subscribe = useCallback<SubscribeToRepoChanges>((listener) => {
    listenersRef.current.add(listener)
    return () => listenersRef.current.delete(listener)
  }, [])

  useEffect(() => () => {
    if (scope) invalidateRepoResources(scope, resyncTopics())
  }, [scope])

  useEffect(() => {
    if (!repoId || !scope || !eventStreamUrl || !tokenTemplate) {
      return
    }

    const controller = new AbortController()
    const notifyListeners = (event: RepoChangeEvent) => {
      for (const listener of listenersRef.current) {
        try {
          listener(event)
        } catch (error) {
          console.warn('[scope] repository change listener failed', error)
        }
      }
    }
    const onEvent = (event: RepoChangeEvent) => {
      if (event.repo_id === repoId) {
        const summaryPending = repoSummaryChangedBy(event, repoSummaryResource.peek(summaryIdentity))
        if (summaryPending) repoSummaryResource.invalidate(summaryIdentity)
        invalidateRepoResources(scope, topicsOf(event, { summaryPending, versioned }))
      }
      notifyListeners(event)
    }
    const onStreamInterrupted = () => {
      invalidateRepoResources(scope, resyncTopics({ summaryPending: true, versioned }))
      repoSummaryResource.invalidate(summaryIdentity)
      const event: RepoChangeEvent = {
        incarnation_id: 'local-stream-interruption',
        kind: 'Lagged',
        repo_id: repoId,
        version: 0,
      }
      notifyListeners(event)
    }

    void runRepoEventStream({
      connect: (deliver, signal) =>
        streamRepoEvents({ event_stream_url: eventStreamUrl, clerk_token_template: tokenTemplate }, getToken, deliver, signal),
      onEvent,
      onInterrupted: onStreamInterrupted,
      signal: controller.signal,
    })
    return () => controller.abort()
  }, [getToken, repoId, scope, summaryIdentity, eventStreamUrl, tokenTemplate, versioned])

  const acceptedRef = useRef<{ live: RepoLiveState; scope: string } | null>(null)
  useEffect(() => {
    const previous = acceptedRef.current
    acceptedRef.current = live && scope ? { live, scope } : null
    if (!live || !scope || !previous || previous.scope !== scope || previous.live === live) return
    if (previous.live.repo.change_version === live.repo.change_version) {
      invalidateRepoResources(scope, [{ topic: 'requestList' }])
    }
  }, [live, scope])

  return subscribe
}
