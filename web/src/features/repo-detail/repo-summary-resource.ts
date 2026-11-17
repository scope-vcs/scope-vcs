import { HttpError, InvalidApiResponseError } from '../../api/http'
import type { RepoLiveState, RepoParams } from '../../api/types'
import { fetchServerFunction, StaleBuildError } from '../../lib/stale-build'
import { createRepoResource } from './repo-resource'
import { repoResourceScopeRepoId } from './repo-resource-scope'

export type RepoSummaryLoadResult = { live: RepoLiveState } | { unavailable: string }

class RepoSummaryUnavailable extends Error {}

export const repoSummaryResource = createRepoResource<RepoLiveState>({
  invalidatedBy: ['summary'],
  supersedes: ['requestList'],
  keyIn: (scope, { snapshot }) => snapshot.value?.repo.id === repoResourceScopeRepoId(scope) ? [] : null,
  affectedBy: ({ version }, _key, { snapshot }) => version === undefined
    || snapshot.value === null
    || snapshot.value.repo.access.actor === 'Public'
    || version > snapshot.value.repo.change_version,
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
