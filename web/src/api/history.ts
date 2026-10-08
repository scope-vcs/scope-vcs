import { createApiClient } from '@/api/client'
import { renderReviewFileDiff } from '@/features/review/review-file-diff-prerender'
import { parseHistoryFeed } from './history-inputs'
import { parseViewId } from './repo-views'
import type {
  HistoryEntryDetailInput,
  HistoryEntryFileDiffInput,
  HistoryPageInput,
  ReviewFileDiff,
} from './types'
import type {
  HistoryEntryDetailResponse,
  HistoryPageResponse,
} from './types.generated'
import { ApiRouteTemplates, buildApiPath } from './types.generated'
import {
  HistoryEntryDetailResponseValidator,
  HistoryPageResponseValidator,
  ReviewFileDiffResponseValidator,
} from './validators.generated'
export {
  parseHistoryEntryDetailInput,
  parseHistoryEntryFileDiffInput,
  parseHistoryPageInput,
} from './history-inputs'

export async function loadHistoryPageForRequest(
  data: HistoryPageInput,
  signal?: AbortSignal,
): Promise<HistoryPageResponse> {
  const query = new URLSearchParams()
  if (data.view) query.set('view', parseViewId(data.view))
  if (data.before) query.set('before', data.before)
  query.set('feed', parseHistoryFeed(data.feed))

  return createApiClient().get(
    `${buildApiPath(ApiRouteTemplates.repoHistory, {
      owner: data.owner,
      repo: data.repo,
    })}?${query}`,
    HistoryPageResponseValidator,
    { auth: 'optional', signal },
  )
}

export async function loadHistoryEntryForRequest(
  data: HistoryEntryDetailInput,
): Promise<HistoryEntryDetailResponse> {
  const query = new URLSearchParams()
  if (data.view) query.set('view', parseViewId(data.view))

  return createApiClient().get(
    `${buildApiPath(ApiRouteTemplates.repoHistoryEntry, {
      owner: data.owner,
      repo: data.repo,
      entry_id: data.entry,
    })}?${query}`,
    HistoryEntryDetailResponseValidator,
    { auth: 'optional' },
  )
}

export async function loadHistoryEntryFileDiffForRequest(
  data: HistoryEntryFileDiffInput,
  signal?: AbortSignal,
): Promise<ReviewFileDiff> {
  const query = new URLSearchParams({
    view: parseViewId(data.view),
    path: data.path,
  })

  if (data.visibility_change) query.set('visibility_change', data.visibility_change)

  const diff = await createApiClient().get(
    `${buildApiPath(ApiRouteTemplates.repoHistoryEntryFileDiff, {
      owner: data.owner,
      repo: data.repo,
      entry_id: data.entry,
    })}?${query}`,
    ReviewFileDiffResponseValidator,
    { auth: 'optional', signal },
  )

  return renderReviewFileDiff(diff, signal)
}
