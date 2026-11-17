import { HttpError, InvalidApiResponseError } from '../../api/http'
import type { RepoLiveState, RepoParams } from '../../api/types'
import type { RepoChangeEvent } from '../../api/types.generated'
import { createCachedResource } from '../../lib/cached-resource'
import { fetchServerFunction, StaleBuildError } from '../../lib/stale-build'

export type RepoSummaryLoadResult = { live: RepoLiveState } | { unavailable: string }

class RepoSummaryUnavailable extends Error {}

export const repoSummaryResource = createCachedResource<RepoLiveState>({
  maxEntries: 8,
  coalesceInvalidations: true,
})

export function repoSummaryIdentity({ owner, repo }: RepoParams) {
  return `${owner}/${repo}`
}

export function isRetryableRepoSummaryError(error: unknown) {
  if (error instanceof HttpError) return error.response.retryable
  if (error instanceof InvalidApiResponseError) return error.status >= 500
  return error instanceof RepoSummaryUnavailable || error instanceof TypeError
}

export const fetchRepoSummary: typeof fetch = async (input, init) => {
  const response = await fetchServerFunction(input, init)
  if (response.status >= 500) {
    await response.body?.cancel()
    throw new RepoSummaryUnavailable('Repository refresh is temporarily unavailable.')
  }
  return response
}

export function acceptRepoSummary(result: RepoSummaryLoadResult) {
  if ('live' in result) return result.live
  throw new RepoSummaryUnavailable(result.unavailable)
}

export function loadRepoSummary(identity: string, load: (signal: AbortSignal) => Promise<RepoLiveState>) {
  const { value, error } = repoSummaryResource.getSnapshot(identity)
  const retained = error === null || error instanceof StaleBuildError || isRetryableRepoSummaryError(error)
  if (value && retained) return Promise.resolve(value)
  return repoSummaryResource.load(identity, '', load)
}

export function forgetRepoSummary(identity: string) {
  repoSummaryResource.removeMatching((retained) => retained === identity)
}

export function repoSummaryChangedBy(event: RepoChangeEvent, summary: RepoLiveState | null) {
  const { kind } = event
  if (kind === 'GitHubWorkflowRunsChanged') return false
  if (typeof kind === 'object' && ('GitHubWorkflowRunChanged' in kind || 'RequestTimelineChanged' in kind)) return false
  if (kind === 'Connected' || kind === 'Lagged' || event.version === 0) return true
  return summary === null
    || summary.repo.access.actor === 'Public'
    || event.version > summary.repo.change_version
}
