import { createApiClient } from '@/api/client'
import { requestRoute } from './paths'
import { renderReviewFileDiff } from '@/features/review/review-file-diff-prerender'
import type { ApproveRequestChecksInput, ReviewFileDiff, RequestParams } from './types'
import type {
  RequestChecksResponse,
  RequestQueuePageResponse,
  RequestRatingResponse,
  RequestRatingsResponse,
  RequestRevisionListResponse,
} from './types.generated'
import { ApiRouteTemplates, buildApiPath } from './types.generated'
import {
  RequestChecksResponseValidator,
  RequestQueuePageResponseValidator,
  RequestRatingResponseValidator,
  RequestRatingsResponseValidator,
  RequestRevisionListResponseValidator,
  RequestStateResponseValidator,
  ReviewFileDiffResponseValidator,
} from './validators.generated'
import type { LoadRequestQueueInput } from './request-queue-input'

export async function loadRequestQueueForRequest(
  data: LoadRequestQueueInput,
  signal?: AbortSignal,
): Promise<RequestQueuePageResponse> {
  return createApiClient().get(
    requestQueuePath(data),
    RequestQueuePageResponseValidator,
    { auth: 'optional', signal },
  )
}

export async function approveRequestChecks(
  data: ApproveRequestChecksInput,
): Promise<RequestChecksResponse> {
  return createApiClient().post(
    requestRoute(ApiRouteTemplates.repoRequestChecksApprove, data),
    RequestChecksResponseValidator,
    { auth: 'required', body: { expected_head_oid: data.expected_head_oid } },
  )
}

export type RateRequestInput = RequestParams & {
  score: number
  reason: string
}

export async function loadRequestRatingsForRequest(
  data: RequestParams,
  signal?: AbortSignal,
): Promise<RequestRatingsResponse> {
  return createApiClient().get(
    requestRoute(ApiRouteTemplates.repoRequestRatings, data),
    RequestRatingsResponseValidator,
    { auth: 'optional', signal },
  )
}

export async function rateRequestForRequest(
  data: RateRequestInput,
): Promise<RequestRatingResponse> {
  return createApiClient().post(
    requestRoute(ApiRouteTemplates.repoRequestRatings, data),
    RequestRatingResponseValidator,
    {
      auth: 'required',
      body: { reason: data.reason, score: data.score },
    },
  )
}

export async function loadRequestRevisionsForRequest(
  data: RequestParams & { commit_oid?: string; revision_id?: string },
): Promise<RequestRevisionListResponse> {
  const search = new URLSearchParams()
  if (data.revision_id) search.set('revision', data.revision_id)
  if (data.commit_oid) search.set('commit', data.commit_oid)
  const path = requestRoute(ApiRouteTemplates.repoRequestRevisions, data)
  return createApiClient().get(
    search.size > 0 ? `${path}?${search}` : path,
    RequestRevisionListResponseValidator,
    { auth: 'optional' },
  )
}

export type LoadRequestRevisionCommitInput = RequestParams & {
  commit_oid: string
  revision_id: string
}

export async function loadRequestRevisionCommitFileDiffForRequest(
  data: LoadRequestRevisionCommitInput & { path: string },
  signal?: AbortSignal,
): Promise<ReviewFileDiff> {
  const path = requestRevisionCommitRoute(
    ApiRouteTemplates.repoRequestRevisionCommitFileDiff,
    data,
  )
  const diff = await createApiClient().get(
    `${path}?path=${encodeURIComponent(data.path)}`,
    ReviewFileDiffResponseValidator,
    { auth: 'optional', signal },
  )

  return renderReviewFileDiff(diff, signal)
}

function requestQueuePath(data: LoadRequestQueueInput) {
  const path = buildApiPath(ApiRouteTemplates.repoRequestQueue, {
    owner: data.owner,
    repo: data.repo,
  })
  const search = new URLSearchParams({ section: data.section })
  if (data.cursor) {
    search.set('cursor', data.cursor)
  }
  if (data.search) {
    search.set('search', data.search)
  }
  if (data.view) {
    search.set('view', data.view)
  }
  return `${path}?${search}`
}

function requestRevisionCommitRoute(
  template: string,
  data: LoadRequestRevisionCommitInput,
) {
  return buildApiPath(template, {
    commit_oid: data.commit_oid,
    owner: data.owner,
    repo: data.repo,
    request_id: data.request_id,
    revision_id: data.revision_id,
  })
}

export async function loadRequestStateForRequest(
  data: RequestParams,
  signal?: AbortSignal,
) {
  return createApiClient().get(
    requestRoute(ApiRouteTemplates.repoRequestState, data),
    RequestStateResponseValidator,
    { auth: 'optional', signal },
  )
}
