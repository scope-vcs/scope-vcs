import type { RepoLiveState } from '@/api/types'
import type { RepoChangeEvent } from '@/api/types.generated'
import {
  browserScheduler,
  createRefreshCoordinator,
  type RefreshScheduler,
} from '../../lib/refresh-coordinator'
import { useAuth } from '@clerk/tanstack-react-start'
import { useCallback, useEffect, useRef } from 'react'
import { runRepoEventStream, streamRepoEvents } from './repo-event-stream'
import { repoResourceScope } from './repo-resource-scope'
import { invalidateRepoResources, invalidateRepoSummaryResources } from './repo-resource-invalidation'

/** A forced refresh ignores versions; a versioned one is dropped once applied. */
type RepoRefreshRequest = { force: boolean; version: number | null }
type RepoChangeListener = (event: RepoChangeEvent) => void
export type SubscribeToRepoChanges = (
  listener: RepoChangeListener,
) => () => void

type RepoRefreshCoordinator = {
  onEvent: (event: RepoChangeEvent) => void
  onStreamInterrupted: () => void
  onSummary: (refreshId: string, version: number) => void
  stop: () => void
}

export function useRepoLiveRefresh(
  live: RepoLiveState | null,
  invalidate: () => Promise<unknown>,
  refreshId: string,
) {
  const { getToken, isLoaded, userId } = useAuth()
  const scope = live && isLoaded ? repoResourceScope(live.repo, userId ?? null) : null
  const repoId = live?.repo.id
  const version = live?.repo.change_version ?? 0
  const versioned = live?.repo.access.actor !== 'Public'
  const eventStreamUrl = live?.event_stream_url
  const tokenTemplate = live?.clerk_token_template
  const coordinatorRef = useRef<RepoRefreshCoordinator | null>(null)
  const listenersRef = useRef(new Set<RepoChangeListener>())
  const subscribe = useCallback<SubscribeToRepoChanges>((listener) => {
    listenersRef.current.add(listener)
    return () => listenersRef.current.delete(listener)
  }, [])

  useEffect(() => () => {
    // Once we leave this repository/access scope, its disconnected interval
    // needs reconciliation on return, including public views without versions.
    if (scope) invalidateRepoResources(scope)
  }, [scope])

  useEffect(() => {
    if (!repoId || !scope || !eventStreamUrl || !tokenTemplate) {
      return
    }

    const controller = new AbortController()
    const coordinator = createRepoRefreshCoordinator({
      initialVersion: 0,
      invalidate,
      repoId,
      schedule: browserScheduler,
      versioned,
      onSummaryRefresh: () => invalidateRepoSummaryResources(scope),
    })
    coordinatorRef.current = coordinator
    const notifyListeners = (event: RepoChangeEvent) => {
      for (const listener of listenersRef.current) {
        try {
          listener(event)
        } catch {
          // A broken page subscriber must not tear down the shared stream.
        }
      }
    }
    const onEvent = (event: RepoChangeEvent) => {
      if (event.repo_id === repoId) {
        // Connected also covers changes committed after an interruption refresh.
        invalidateRepoResources(scope, event)
      }
      coordinator.onEvent(event)
      notifyListeners(event)
    }
    const onStreamInterrupted = () => {
      invalidateRepoResources(scope)
      coordinator.onStreamInterrupted()
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
    return () => {
      coordinatorRef.current = null
      coordinator.stop()
      controller.abort()
    }
  }, [getToken, invalidate, repoId, scope, eventStreamUrl, tokenTemplate, versioned])

  useEffect(() => {
    coordinatorRef.current?.onSummary(refreshId, version)
  }, [refreshId, version, scope, getToken, invalidate, repoId, eventStreamUrl, tokenTemplate, versioned])

  return subscribe
}

export function createRepoRefreshCoordinator({
  initialVersion,
  invalidate,
  repoId,
  schedule,
  versioned,
  onSummaryRefresh = () => {},
}: {
  initialVersion: number
  invalidate: () => Promise<unknown>
  repoId: string
  schedule: RefreshScheduler
  versioned: boolean
  onSummaryRefresh?: () => void
}): RepoRefreshCoordinator {
  let highestAppliedVersion = initialVersion
  let lastSummaryId: string | null = null
  const coordinator = createRefreshCoordinator<RepoRefreshRequest>({
    merge: (pending, next) => ({
      force: pending.force || next.force,
      version: next.version === null
        ? pending.version
        : Math.max(pending.version ?? next.version, next.version),
    }),
    refresh: async (request) => {
      await invalidate()
      if (request.version !== null) {
        highestAppliedVersion = Math.max(highestAppliedVersion, request.version)
      }
    },
    schedule,
    shouldRefresh: (request) =>
      request.force || (request.version !== null && request.version > highestAppliedVersion),
  })
  const requestRefresh = (version: number | null) =>
    coordinator.request({ force: version === null, version })

  return {
    onEvent(event) {
      if (
        event.repo_id !== repoId ||
        typeof event.kind === 'object' &&
          ('RequestTimelineChanged' in event.kind || 'RunChanged' in event.kind)
      ) {
        return
      }
      if (event.kind === 'Connected' || event.kind === 'Lagged' || !versioned || event.version === 0) {
        requestRefresh(null)
      } else if (event.version > highestAppliedVersion) {
        requestRefresh(event.version)
      }
    },
    onSummary(refreshId, version) {
      highestAppliedVersion = Math.max(highestAppliedVersion, version)
      if (lastSummaryId === refreshId) return
      const hadSummary = lastSummaryId !== null
      lastSummaryId = refreshId
      if (hadSummary) onSummaryRefresh()
    },
    onStreamInterrupted: () => requestRefresh(null),
    stop: coordinator.stop,
  }
}
