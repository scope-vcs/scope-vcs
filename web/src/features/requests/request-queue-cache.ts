import type { RequestQueuePageResponse, RequestQueueSection } from '../../api/types.generated'
import { createCachedResource } from '../../lib/cached-resource'
import { appendQueuePage, REQUEST_QUEUE_SECTION_ORDER, type RequestQueuePages, type RequestQueueViewState } from './request-list-model'

export type LoadRequestQueuePage = (section: RequestQueueSection, cursor: string | null, search: string | null, signal?: AbortSignal) => Promise<RequestQueuePageResponse>

export const requestQueueResource = createCachedResource<RequestQueueViewState>({
  maxEntries: 12,
  maxWeight: 4 * 1024 * 1024,
  weightOf: (value) => JSON.stringify(value).length * 2,
})

// A refresh refills the visible depth of each section. Navigation and incoming
// activity therefore keep already-loaded rows, including searched pages.
export async function refreshRequestQueue(key: string, load: LoadRequestQueuePage, signal: AbortSignal): Promise<RequestQueueViewState> {
  const previous = requestQueueResource.peek(key)
  const query = previous?.query ?? ''
  const entries = await Promise.all(REQUEST_QUEUE_SECTION_ORDER.map(async (section) => {
    let page = await load(section, null, query || null, signal)
    const depth = previous?.pages[section].requests.length ?? 0
    const cursors = new Set<string>()
    while (page.next_cursor && page.requests.length < depth && !signal.aborted) {
      if (cursors.has(page.next_cursor)) break
      cursors.add(page.next_cursor)
      page = appendQueuePage(page, await load(section, page.next_cursor, query || null, signal))
    }
    return [section, page] as const
  }))
  return { pages: Object.fromEntries(entries) as RequestQueuePages, query }
}

export async function searchRequestQueue(key: string, query: string, load: LoadRequestQueuePage) {
  const normalized = query.trim()
  const snapshot = requestQueueResource.getSnapshot(key)
  if (snapshot.value?.query === normalized && !snapshot.error) return
  requestQueueResource.invalidate(key)
  await requestQueueResource.ensure(key, snapshot.version ?? '', async (signal) => {
    const entries = await Promise.all(REQUEST_QUEUE_SECTION_ORDER.map(async (section) =>
      [section, await load(section, null, normalized || null, signal)] as const))
    return { pages: Object.fromEntries(entries) as RequestQueuePages, query: normalized }
  })
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
      [section]: appendQueuePage(current.pages[section], await load(section, cursor, current.query || null, signal)),
    },
  }))
}
