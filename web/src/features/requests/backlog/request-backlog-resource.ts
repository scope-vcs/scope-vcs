import type { RequestBacklogQuery } from '../../../api/request-backlog-input'
import type { RequestBacklogPageResponse, ViewId } from '../../../api/types.generated'
import { createRepoResource, repoResourceKey } from '../../repo-detail/repo-resource'
import { appendBacklogPage } from './request-backlog-model'

export type LoadRequestBacklogPage = (cursor: string | null, signal?: AbortSignal) => Promise<RequestBacklogPageResponse>

export const requestBacklogResource = createRepoResource<RequestBacklogPageResponse>({
  invalidatedBy: ['requestList'],
  maxEntries: 12,
  maxWeight: 2 * 1024 * 1024,
  weightOf: (value) => JSON.stringify(value).length * 2,
})

export function requestBacklogIdentity(scope: string, view: ViewId, query: RequestBacklogQuery) {
  return repoResourceKey(scope, view, query.working, query.sort, query.search)
}

export function requestBacklogTotalIdentity(scope: string, view: ViewId) {
  return repoResourceKey(scope, view, 'total')
}

export async function refreshRequestBacklog(key: string, load: LoadRequestBacklogPage, signal: AbortSignal) {
  const depth = requestBacklogResource.peek(key)?.requests.length ?? 0
  let page = await load(null, signal)
  const cursors = new Set<string>()
  while (page.next_cursor && page.requests.length < depth && !signal.aborted) {
    if (cursors.has(page.next_cursor)) break
    cursors.add(page.next_cursor)
    page = appendBacklogPage(page, await load(page.next_cursor, signal))
  }
  return page
}

export async function loadMoreRequestBacklog(key: string, load: LoadRequestBacklogPage) {
  const snapshot = requestBacklogResource.getSnapshot(key)
  const current = snapshot.value
  const cursor = current?.next_cursor
  if (!current || !cursor || snapshot.pending || snapshot.stale) return
  requestBacklogResource.invalidate(key)
  await requestBacklogResource.ensure(key, snapshot.version ?? '', async (signal) =>
    appendBacklogPage(current, await load(cursor, signal)))
}
