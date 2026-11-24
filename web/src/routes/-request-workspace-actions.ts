import { parseRequestAttentionInput } from '@/api/request-attention-input'
import { updateRequestAttentionForRequest } from '@/features/requests/workspace/request-attention-api'
import { loadRequestBacklogForRequest, loadRequestQueueForRequest } from '@/api/requests'
import { parseLoadRequestBacklogInput, type RequestBacklogQuery } from '@/api/request-backlog-input'
import { parseLoadRequestQueueInput } from '@/api/request-queue-input'
import type { RepoParams } from '@/api/types'
import type { ViewId } from '@/api/types.generated'
import { DEFAULT_REQUEST_BACKLOG_QUERY } from '@/features/requests/backlog/request-backlog-model'
import type { LoadRequestBacklogPage } from '@/features/requests/backlog/request-backlog-resource'
import type { LoadRequestQueuePage } from '@/features/requests/workspace/request-queue-cache'
import { createServerFn } from '@tanstack/react-start'

export const loadRequestQueuePage = createServerFn({ method: 'GET' })
  .validator(parseLoadRequestQueueInput)
  .handler(({ data }) => loadRequestQueueForRequest(data))

const loadRequestBacklogPage = createServerFn({ method: 'GET' })
  .validator(parseLoadRequestBacklogInput)
  .handler(({ data }) => loadRequestBacklogForRequest(data))

export const updateRequestAttention = createServerFn({ method: 'POST' })
  .validator(parseRequestAttentionInput)
  .handler(({ data }) => updateRequestAttentionForRequest(data))

export function requestQueuePageLoader({ owner, repo }: RepoParams, view: ViewId): LoadRequestQueuePage {
  return (section, cursor, signal) => loadRequestQueuePage({
    data: { owner, repo, section, cursor, search: null, view },
    signal,
  })
}

export function requestBacklogPageLoader(
  { owner, repo }: RepoParams,
  view: ViewId,
  query: RequestBacklogQuery,
  limit: number | null = null,
): LoadRequestBacklogPage {
  return (cursor, signal) => loadRequestBacklogPage({
    data: { owner, repo, ...query, cursor, limit, view },
    signal,
  })
}

export function requestBacklogTotalLoader(params: RepoParams, view: ViewId) {
  return requestBacklogPageLoader(params, view, DEFAULT_REQUEST_BACKLOG_QUERY, 1)
}
