import { useCallback, useSyncExternalStore } from 'react'
import type { ReviewDiffRenderBudget, ReviewFileDiff } from '@/api/types'
import type { CachedResourceStore } from '@/lib/cached-resource'
import { type CachedResource, useCachedResource } from '@/lib/use-cached-resource'

export type ReviewDiffFullRender = {
  error: string | null
  pending: boolean
  start: () => void
}

export type ReviewFileDiffResource = CachedResource<ReviewFileDiff> & {
  fullRender: ReviewDiffFullRender | null
}

export function useReviewFileDiffResource({
  fallbackError,
  identity,
  load,
  resource,
}: {
  fallbackError: string
  identity: string | null
  load: (budget: ReviewDiffRenderBudget, signal: AbortSignal) => Promise<ReviewFileDiff>
  resource: CachedResourceStore<ReviewFileDiff>
}): ReviewFileDiffResource {
  const subscribe = useCallback((listener: () => void) => identity
    ? resource.subscribe(identity, listener)
    : () => {}, [identity, resource])
  const requestedBudget = useSyncExternalStore(
    subscribe,
    () => identity ? resource.getSnapshot(identity).version : null,
    () => null,
  )
  const budget: ReviewDiffRenderBudget = requestedBudget === 'full' ? 'full' : 'standard'
  const loadBudget = useCallback((signal: AbortSignal) => load(budget, signal), [budget, load])
  const diff = useCachedResource({
    fallbackError,
    identity,
    load: loadBudget,
    resource,
    version: budget,
  })
  const start = useCallback(() => {
    if (!identity) return
    resource.invalidate(identity)
    void resource.ensure(identity, 'full', (signal) => load('full', signal))
  }, [identity, load, resource])

  const presentation = diff.value?.presentation
  const fullRender = presentation?.kind === 'omitted' && presentation.budget === 'standard'
    ? {
        error: budget === 'full' ? diff.error : null,
        pending: budget === 'full' && diff.refreshing,
        start,
      }
    : null
  return { ...diff, fullRender }
}
