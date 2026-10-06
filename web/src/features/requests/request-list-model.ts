import type { RequestQueueItemResponse, RequestQueuePageResponse, RequestQueueSection } from '../../api/types.generated'

export const REQUEST_QUEUE_SECTION_ORDER = ['active', 'unclaimed', 'set_aside', 'done'] as const satisfies readonly RequestQueueSection[]
export type RequestQueuePages = Record<RequestQueueSection, RequestQueuePageResponse>
export type RequestQueueViewState = { pages: RequestQueuePages; query: string; requestedQuery: string }

function appendRequestPage(current: RequestQueueItemResponse[], incoming: RequestQueueItemResponse[]) {
  const rows = new Map(current.map((item) => [item.request.id, item]))
  for (const item of incoming) rows.set(item.request.id, item)
  return [...rows.values()]
}

export function appendQueuePage(current: RequestQueuePageResponse, incoming: RequestQueuePageResponse): RequestQueuePageResponse {
  return { ...incoming, requests: appendRequestPage(current.requests, incoming.requests) }
}

export function nextRequestAttentionAt(pages: RequestQueuePages): number | null {
  const times = REQUEST_QUEUE_SECTION_ORDER.flatMap((section) => {
    const time = pages[section].next_attention_at_unix
    return time === null ? [] : [time]
  })
  return times.length ? Math.min(...times) : null
}
