import { isNotFoundError } from '@/api/http'
import { loadRepoLiveStateForRequest } from '@/api/repo-detail'
import { parseRepoParams } from '@/api/repo-params'
import type { RepoParams } from '@/api/types'
import {
  acceptRepoSummary,
  fetchRepoSummary,
  isRetryableRepoSummaryError,
  type RepoSummaryLoadResult,
} from '@/features/repo-detail/repo-summary-resource'
import { notFound } from '@tanstack/react-router'
import { createServerFn } from '@tanstack/react-start'

const loadRepoLiveState = createServerFn({ method: 'GET' })
  .validator(parseRepoParams)
  .handler(async ({ data }): Promise<RepoSummaryLoadResult> => {
    try {
      return { live: await loadRepoLiveStateForRequest(data) }
    } catch (error) {
      if (isNotFoundError(error)) notFound({ throw: true })
      if (isRetryableRepoSummaryError(error)) {
        return { unavailable: 'Repository refresh is temporarily unavailable.' }
      }
      throw error
    }
  })

export async function loadRepoSummaryState({ owner, repo }: RepoParams, signal: AbortSignal) {
  return acceptRepoSummary(await loadRepoLiveState({ data: { owner, repo }, fetch: fetchRepoSummary, signal }))
}
