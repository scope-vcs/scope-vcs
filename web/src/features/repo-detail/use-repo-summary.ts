import type { RepoLiveState } from '../../api/types'
import { StaleBuildError } from '../../lib/stale-build'
import { useCachedResource } from '../../lib/use-cached-resource'
import { useEffect } from 'react'
import { isRetryableRepoSummaryError, repoSummaryResource } from './repo-summary-resource'

const FAILED_REFRESH_RETRY_MS = 2_000

export function useRepoSummary({
  identity,
  initial,
  load,
  onRejected,
}: {
  identity: string
  initial: RepoLiveState
  load: (signal: AbortSignal) => Promise<RepoLiveState>
  onRejected: () => void
}) {
  const summary = useCachedResource({
    fallbackError: 'Repository refresh is temporarily unavailable.',
    identity,
    initialValue: initial,
    load,
    resource: repoSummaryResource,
  })
  const failure = repoSummaryResource.getSnapshot(identity).error
  const { retry } = summary
  useEffect(() => {
    if (failure === null || failure instanceof StaleBuildError) return
    if (!isRetryableRepoSummaryError(failure)) {
      onRejected()
      return
    }
    const timer = setTimeout(retry, FAILED_REFRESH_RETRY_MS)
    return () => clearTimeout(timer)
  }, [failure, onRejected, retry])
  return summary.value ?? initial
}
