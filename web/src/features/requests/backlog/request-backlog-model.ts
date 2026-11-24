import {
  parseRequestBacklogQuery,
  type RequestBacklogQuery,
} from '../../../api/request-backlog-input'
import type {
  RequestBacklogItemResponse,
  RequestBacklogPageResponse,
  RequestBacklogSort,
  RequestBacklogWorking,
} from '../../../api/types.generated'

export const REQUEST_BACKLOG_NAME = 'Backlog'

export const REQUEST_BACKLOG_WORKING_LABELS = {
  anyone: 'Working: anyone',
  nobody: 'Working: nobody',
  me: 'Working: me',
} as const satisfies Record<RequestBacklogWorking, string>

export const REQUEST_BACKLOG_SORT_LABELS = {
  oldest: 'Oldest first',
  newest: 'Newest first',
} as const satisfies Record<RequestBacklogSort, string>

export const DEFAULT_REQUEST_BACKLOG_QUERY = parseRequestBacklogQuery({})

export type RequestBacklogRouteSearch = {
  q?: string
  working?: RequestBacklogWorking
  sort?: RequestBacklogSort
}

export function requestBacklogQuery(search: { q?: unknown; working?: unknown; sort?: unknown }): RequestBacklogQuery {
  return parseRequestBacklogQuery({
    search: typeof search.q === 'number' ? String(search.q) : search.q,
    working: search.working,
    sort: search.sort,
  })
}

export function requestBacklogQueryFor(query: RequestBacklogQuery, signedOut: boolean): RequestBacklogQuery {
  return signedOut && query.working === 'me'
    ? { ...query, working: DEFAULT_REQUEST_BACKLOG_QUERY.working }
    : query
}

export function requestBacklogRouteSearch(query: RequestBacklogQuery): RequestBacklogRouteSearch {
  const defaults = DEFAULT_REQUEST_BACKLOG_QUERY
  return {
    q: query.search || undefined,
    working: query.working === defaults.working ? undefined : query.working,
    sort: query.sort === defaults.sort ? undefined : query.sort,
  }
}

export function appendBacklogPage(
  current: RequestBacklogPageResponse,
  incoming: RequestBacklogPageResponse,
): RequestBacklogPageResponse {
  const rows = new Map(current.requests.map((item) => [item.id, item]))
  for (const item of incoming.requests) rows.set(item.id, item)
  return { ...incoming, requests: [...rows.values()] }
}

export function requestBacklogCountLabel(count: number) {
  return `${count} ${count === 1 ? 'request' : 'requests'}`
}

export function requestBacklogSource(item: RequestBacklogItemResponse) {
  return `${item.asker_login}, GitHub #${item.github_issue_number}`
}

export function requestBacklogRepliesLabel(count: number) {
  return `${count} ${count === 1 ? 'reply' : 'replies'}`
}

export type BacklogCloseFailures = ReadonlyMap<string, string>

export async function closeBacklogRequests(
  ids: readonly string[],
  close: (id: string) => Promise<unknown>,
  concurrency = 4,
): Promise<BacklogCloseFailures> {
  const failures = new Map<string, string>()
  const queue = [...ids]
  async function work() {
    for (let id = queue.shift(); id !== undefined; id = queue.shift()) {
      try {
        await close(id)
      } catch (cause) {
        failures.set(id, cause instanceof Error && cause.message.trim() ? cause.message : 'Could not close this request.')
      }
    }
  }
  await Promise.all(Array.from({ length: Math.min(concurrency, queue.length) }, work))
  return failures
}
