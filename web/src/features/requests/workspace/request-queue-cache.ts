import type { RequestQueuePageResponse, RequestQueueSection, ViewId } from '../../../api/types.generated'
import { createRepoResource, repoResourceKey } from '../../repo-detail/repo-resource'
import { appendQueuePage, REQUEST_QUEUE_SECTION_ORDER, type RequestQueuePages, type RequestQueueViewState } from './request-list-model'

export type LoadRequestQueuePage = (section: RequestQueueSection, cursor: string | null, signal?: AbortSignal) => Promise<RequestQueuePageResponse>

export const requestQueueResource = createRepoResource<RequestQueueViewState>({
  invalidatedBy: ['requestList'],
  maxEntries: 12,
  maxWeight: 4 * 1024 * 1024,
  weightOf: (value) => JSON.stringify(value).length * 2,
})

export function requestQueueIdentity(scope: string, view: ViewId, search: string | null = null) {
  return repoResourceKey(scope, view, ...search === null ? [] : [search])
}

export async function refreshRequestQueue(key: string | null, load: LoadRequestQueuePage, signal: AbortSignal): Promise<RequestQueueViewState> {
  const previous = key === null ? null : requestQueueResource.peek(key)
  const entries = await Promise.all(REQUEST_QUEUE_SECTION_ORDER.map(async (section) => {
    let page = await load(section, null, signal)
    const depth = previous?.pages[section].requests.length ?? 0
    const cursors = new Set<string>()
    while (page.next_cursor && page.requests.length < depth && !signal.aborted) {
      if (cursors.has(page.next_cursor)) break
      cursors.add(page.next_cursor)
      page = appendQueuePage(page, await load(section, page.next_cursor, signal))
    }
    return [section, page] as const
  }))
  return { pages: Object.fromEntries(entries) as RequestQueuePages }
}

export async function loadMoreRequestQueue(key: string, section: RequestQueueSection, load: LoadRequestQueuePage) {
  const snapshot = requestQueueResource.getSnapshot(key)
  const current = snapshot.value
  const cursor = current?.pages[section].next_cursor
  if (!current || !cursor || snapshot.pending || snapshot.stale) return
  requestQueueResource.invalidate(key)
  await requestQueueResource.ensure(key, snapshot.version ?? '', async (signal) => ({
    ...current,
    pages: {
      ...current.pages,
      [section]: appendQueuePage(current.pages[section], await load(section, cursor, signal)),
    },
  }))
}
