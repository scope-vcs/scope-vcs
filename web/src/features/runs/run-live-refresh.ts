import type {
  RepoChangeEvent,
  RunChangeKind,
} from '@/api/types.generated'
import {
  browserScheduler,
  createRefreshCoordinator,
  type RefreshScheduler,
} from '../../lib/refresh-coordinator'
import { useCallback, useEffect, useRef } from 'react'
import {
  useRepoChangeSubscription,
  useRepoLayout,
} from '../repo-detail/repo-layout-context'

const RECONCILIATION_INTERVAL_MS = 30_000
const REFRESH_TIMEOUT_MS = 15_000

export type RunRefreshReason = RunChangeKind | 'Recovery' | 'Manual' | 'Lagged'
export type RunRefresh = (
  reasons: ReadonlySet<RunRefreshReason>,
  signal: AbortSignal,
) => Promise<unknown>

type RunRefreshCoordinator = {
  onEvent: (event: RepoChangeEvent) => void
  requestRefresh: (reason?: RunRefreshReason) => void
  stop: () => void
}

export function useRunLiveRefresh({
  acceptedChanges,
  mutable,
  refresh,
  runId,
}: {
  acceptedChanges: readonly RunChangeKind[]
  mutable: boolean
  refresh: RunRefresh
  runId?: string
}) {
  const { repo } = useRepoLayout()
  const refreshRef = useRef(refresh)
  const mutableRef = useRef(mutable)
  useEffect(() => {
    refreshRef.current = refresh
    mutableRef.current = mutable
  }, [mutable, refresh])

  const coordinatorRef = useRef<RunRefreshCoordinator | null>(null)

  useEffect(() => {
    const coordinator = createRunRefreshCoordinator({
      acceptedChanges,
      refresh: (reasons, signal) => refreshRef.current(reasons, signal),
      repoId: repo.id,
      runId,
      schedule: browserScheduler,
    })
    coordinatorRef.current = coordinator
    return () => {
      coordinatorRef.current = null
      coordinator.stop()
    }
  }, [acceptedChanges, repo.id, runId])

  const onEvent = useCallback((event: RepoChangeEvent) => {
    if (mutableRef.current) coordinatorRef.current?.onEvent(event)
  }, [])
  useRepoChangeSubscription(onEvent)

  useEffect(() => {
    if (!mutable) return
    const reconcile = () => coordinatorRef.current?.requestRefresh('Recovery')
    const onFocus = () => {
      if (document.visibilityState === 'visible') reconcile()
    }
    window.addEventListener('focus', onFocus)
    window.addEventListener('online', reconcile)
    document.addEventListener('visibilitychange', onFocus)
    const interval = window.setInterval(
      reconcile,
      RECONCILIATION_INTERVAL_MS,
    )
    return () => {
      window.removeEventListener('focus', onFocus)
      window.removeEventListener('online', reconcile)
      document.removeEventListener('visibilitychange', onFocus)
      window.clearInterval(interval)
    }
  }, [mutable])

  return useCallback(() => coordinatorRef.current?.requestRefresh('Manual'), [])
}

export function createRunRefreshCoordinator({
  acceptedChanges,
  refresh,
  repoId,
  runId,
  schedule,
  timeoutMs = REFRESH_TIMEOUT_MS,
}: {
  acceptedChanges: readonly RunChangeKind[]
  refresh: RunRefresh
  repoId: string
  runId?: string
  schedule: RefreshScheduler
  timeoutMs?: number
}): RunRefreshCoordinator {
  const accepted = new Set(acceptedChanges)
  const coordinator = createRefreshCoordinator<ReadonlySet<RunRefreshReason>>({
    merge: (pending, next) => new Set([...pending, ...next]),
    refresh,
    schedule,
    timeoutMs,
  })
  const requestRefresh = (reason: RunRefreshReason = 'Recovery') =>
    coordinator.request(new Set([reason]))

  return {
    onEvent(event) {
      if (event.repo_id !== repoId) return
      if (event.kind === 'Connected' || event.kind === 'Lagged') {
        requestRefresh(event.kind === 'Lagged' ? 'Lagged' : 'Recovery')
        return
      }
      if (
        typeof event.kind !== 'object' ||
        !('RunChanged' in event.kind)
      ) return
      const changed = event.kind.RunChanged
      if (
        (runId === undefined || changed.run_id === runId) &&
        accepted.has(changed.change)
      ) requestRefresh(changed.change)
    },
    requestRefresh,
    stop: coordinator.stop,
  }
}
