import type { RepoLiveState } from '@/api/types'
import type { RepoChangeEvent } from '@/api/types.generated'
import { useAuth } from '@clerk/tanstack-react-start'
import { useCallback, useEffect, useEffectEvent, useRef } from 'react'
import { eventStreamUrlSince, runRepoEventStream, streamRepoEvents, syncedThrough } from './repo-event-stream'
import { repoResourceScope } from './repo-resource-scope'
import { invalidateRepoResources, resyncTopics, topicsOf } from './repo-resource'

type RepoChangeListener = (event: RepoChangeEvent) => void
export type SubscribeToRepoChanges = (
  listener: RepoChangeListener,
) => () => void

export function useRepoLiveRefresh(live: RepoLiveState | null) {
  const { getToken, isLoaded, userId } = useAuth()
  const scope = live && isLoaded ? repoResourceScope(live.repo, userId ?? null) : null
  const repoId = live?.repo.id
  const versioned = live?.repo.access.actor !== 'Public'
  const eventStreamUrl = live?.event_stream_url
  const tokenTemplate = live?.clerk_token_template
  const listenersRef = useRef(new Set<RepoChangeListener>())
  const syncedRef = useRef<{ scope: string; throughUnixMs: number } | null>(null)
  const loadedAtUnixMs = useEffectEvent(() => live?.loaded_at_unix_ms ?? 0)
  const subscribe = useCallback<SubscribeToRepoChanges>((listener) => {
    listenersRef.current.add(listener)
    return () => listenersRef.current.delete(listener)
  }, [])

  useEffect(() => () => {
    if (scope) invalidateRepoResources(scope, resyncTopics().filter(({ topic }) => topic !== 'summary'))
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
    if (syncedRef.current?.scope !== scope) syncedRef.current = { scope, throughUnixMs: loadedAtUnixMs() }
    const synced = syncedRef.current
    const onEvent = (event: RepoChangeEvent) => {
      synced.throughUnixMs = Math.max(synced.throughUnixMs, syncedThrough(event) ?? 0)
      if (event.repo_id === repoId) invalidateRepoResources(scope, topicsOf(event, { versioned }))
      notifyListeners(event)
    }

    void runRepoEventStream({
      connect: (deliver, signal) => streamRepoEvents(
        { event_stream_url: eventStreamUrlSince(eventStreamUrl, synced.throughUnixMs), clerk_token_template: tokenTemplate },
        getToken,
        deliver,
        signal,
      ),
      onEvent,
      onStopped: () => invalidateRepoResources(scope, resyncTopics({ versioned })),
      signal: controller.signal,
    })
    return () => controller.abort()
  }, [getToken, repoId, scope, eventStreamUrl, tokenTemplate, versioned])

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
