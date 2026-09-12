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
import { invalidateRepoResources } from './repo-resource-invalidation'

/** A forced refresh ignores versions; a versioned one is dropped once applied. */
type RepoRefreshRequest = { force: boolean; version: number | null }
export type RepoChangeListener = (event: RepoChangeEvent) => void
export type SubscribeToRepoChanges = (
  listener: RepoChangeListener,
) => () => void

export type RepoRefreshCoordinator = {
  onEvent: (event: RepoChangeEvent) => void
  onStreamInterrupted: () => void
  stop: () => void
}

export function useRepoLiveRefresh(
  live: RepoLiveState | null,
  invalidate: () => Promise<unknown>,
) {
  const { getToken, isLoaded, userId } = useAuth()
  const scope = live && isLoaded ? repoResourceScope(live.repo, userId ?? null) : null
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
    if (!live || !isLoaded) {
      return
    }

    const controller = new AbortController()
    const coordinator = createRepoRefreshCoordinator({
      initialVersion: live.repo.change_version,
      invalidate,
      repoId: live.repo.id,
      schedule: browserScheduler,
      versioned: usesVersionedRepoChangeEvents(live),
    })
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
      if (scope && event.repo_id === live.repo.id) invalidateRepoResources(scope, event)
      coordinator.onEvent(event)
      notifyListeners(event)
    }
    const onStreamInterrupted = () => {
      if (scope) invalidateRepoResources(scope)
      coordinator.onStreamInterrupted()
      const event: RepoChangeEvent = {
        incarnation_id: 'local-stream-interruption',
        kind: 'Lagged',
        repo_id: live.repo.id,
        version: 0,
      }
      notifyListeners(event)
    }

    void runRepoEventStream({
      connect: (deliver, signal) =>
        streamRepoEvents(live, getToken, deliver, signal),
      onEvent,
      onInterrupted: onStreamInterrupted,
      signal: controller.signal,
    })
    return () => {
      coordinator.stop()
      controller.abort()
    }
  }, [getToken, invalidate, isLoaded, live, scope])

  return subscribe
}

export function createRepoRefreshCoordinator({
  initialVersion,
  invalidate,
  repoId,
  schedule,
  versioned,
}: {
  initialVersion: number
  invalidate: () => Promise<unknown>
  repoId: string
  schedule: RefreshScheduler
  versioned: boolean
}): RepoRefreshCoordinator {
  let highestAppliedVersion = initialVersion
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
        event.kind === 'Connected' ||
        typeof event.kind === 'object' &&
          ('RequestTimelineChanged' in event.kind || 'RunChanged' in event.kind)
      ) {
        return
      }
      if (event.kind === 'Lagged' || !versioned || event.version === 0) {
        requestRefresh(null)
      } else if (event.version > highestAppliedVersion) {
        requestRefresh(event.version)
      }
    },
    onStreamInterrupted: () => requestRefresh(null),
    stop: coordinator.stop,
  }
}

function usesVersionedRepoChangeEvents(live: RepoLiveState) {
  return live.repo.access.actor !== 'Public'
}
