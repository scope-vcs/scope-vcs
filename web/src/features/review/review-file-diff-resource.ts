import { useCallback, useSyncExternalStore } from 'react'
import type { ReviewDiffRenderBudget, ReviewFileDiff } from '../../api/types'
import type { CachedResourceStore } from '../../lib/cached-resource'
import {
  type CachedResource,
  resourceErrorMessage,
  useCachedResource,
} from '../../lib/use-cached-resource'

export type ReviewDiffFullRender = {
  error: string | null
  pending: boolean
  start: () => void
}

export type ReviewFileDiffResource = CachedResource<ReviewFileDiff> & {
  fullRender: ReviewDiffFullRender | null
}

type ReviewFileDiffStore = CachedResourceStore<ReviewFileDiff>
type ReviewFileDiffSnapshot = ReturnType<ReviewFileDiffStore['getSnapshot']>
type LoadReviewFileDiff = (
  budget: ReviewDiffRenderBudget,
  signal: AbortSignal,
) => Promise<ReviewFileDiff>

export function useReviewFileDiffResource({
  fallbackError,
  identity,
  load,
  resource,
}: {
  fallbackError: string
  identity: string | null
  load: LoadReviewFileDiff
  resource: ReviewFileDiffStore
}): ReviewFileDiffResource {
  const subscribe = useCallback((listener: () => void) => identity
    ? resource.subscribe(identity, listener)
    : () => {}, [identity, resource])
  const snapshot = useSyncExternalStore(
    subscribe,
    () => identity ? resource.getSnapshot(identity) : null,
    () => null,
  )
  const budget = reviewFileDiffBudget(snapshot)
  const loadBudget = useCallback((signal: AbortSignal) => load(budget, signal), [budget, load])
  const diff = useCachedResource({
    fallbackError,
    identity,
    load: loadBudget,
    resource,
    version: budget,
  })
  const start = useCallback(() => {
    if (identity) startFullReviewFileDiff(resource, identity, load)
  }, [identity, load, resource])

  return {
    ...diff,
    fullRender: snapshot && reviewFileDiffFullRender(snapshot, fallbackError, start),
  }
}

export function reviewFileDiffBudget(
  snapshot: ReviewFileDiffSnapshot | null,
): ReviewDiffRenderBudget {
  return snapshot?.version === 'full' ? 'full' : 'standard'
}

export function startFullReviewFileDiff(
  resource: ReviewFileDiffStore,
  identity: string,
  load: LoadReviewFileDiff,
) {
  resource.invalidate(identity)
  return resource.ensure(identity, 'full', (signal) => load('full', signal))
}

export function reviewFileDiffFullRender(
  snapshot: ReviewFileDiffSnapshot,
  fallbackError: string,
  start: () => void,
): ReviewDiffFullRender | null {
  const presentation = snapshot.value?.presentation
  if (presentation?.kind !== 'omitted' || presentation.budget !== 'standard') return null
  const requested = reviewFileDiffBudget(snapshot) === 'full'
  return {
    error: requested && snapshot.error !== null
      ? resourceErrorMessage(snapshot.error, fallbackError)
      : null,
    pending: requested && snapshot.pending,
    start,
  }
}
