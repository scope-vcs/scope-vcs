import type { RepoParams } from '@/api/types'
import type { HistoryFeed, RequestQueueSection, ViewId } from '@/api/types.generated'
import { loadHistoryPage } from '@/routes/-repo-history-actions'
import { loadRequestQueuePage } from '@/routes/-request-workspace-actions'
import { appendHistoryPage } from '../history/history-pagination'
import { REQUEST_QUEUE_SECTION_ORDER } from '../requests/request-list-model'
import { repositorySearchResource } from './repository-search-cache'
import type { RepositorySearchResults } from './repository-search-model'

export async function loadRepositorySearch(
  { owner, repo }: RepoParams,
  view: ViewId,
  query: string,
  feed: HistoryFeed,
  signal: AbortSignal,
): Promise<RepositorySearchResults> {
  const search = query || null
  const sections = REQUEST_QUEUE_SECTION_ORDER.filter((section) => search || section !== 'done')
  const [pages, history] = await Promise.all([
    Promise.all(sections.map((section) => loadRequestQueuePage({
      data: { cursor: null, owner, repo, search, section, view },
      signal,
    }))),
    loadHistoryPage({
      data: { before: null, feed, owner, repo, search, view },
      signal,
    }),
  ])
  return {
    history: history.entries,
    historyCursor: history.next_cursor,
    requestCursors: Object.fromEntries(sections.flatMap((section, index) => {
      const cursor = pages[index].next_cursor
      return cursor ? [[section, cursor]] : []
    })),
    requests: pages.flatMap((page) => page.requests),
  }
}

export async function loadMoreRepositorySearchRequests({
  identity,
  params: { owner, repo },
  query,
  view,
}: {
  identity: string
  params: RepoParams
  query: string
  view: ViewId
}) {
  const snapshot = repositorySearchResource.getSnapshot(identity)
  const current = snapshot.value
  if (!current) return
  const pending = Object.entries(current.requestCursors) as [RequestQueueSection, string][]
  if (!pending.length) return
  const pages = await Promise.all(pending.map(([section, cursor]) => loadRequestQueuePage({
    data: { cursor, owner, repo, search: query || null, section, view },
  })))
  const known = new Set(current.requests.map((item) => item.request.id))
  repositorySearchResource.writeIfUnchanged(identity, snapshot, {
    ...current,
    requestCursors: Object.fromEntries(pending.flatMap(([section], index) => {
      const cursor = pages[index].next_cursor
      return cursor ? [[section, cursor]] : []
    })),
    requests: [
      ...current.requests,
      ...pages.flatMap((page) => page.requests).filter((item) => !known.has(item.request.id)),
    ],
  }, snapshot.version ?? '')
}

export async function loadOlderRepositorySearchHistory({
  feed,
  identity,
  params: { owner, repo },
  query,
  view,
}: {
  feed: HistoryFeed
  identity: string
  params: RepoParams
  query: string
  view: ViewId
}) {
  const snapshot = repositorySearchResource.getSnapshot(identity)
  const before = snapshot.value?.historyCursor
  if (!snapshot.value || !before) return
  const page = await loadHistoryPage({
    data: { before, feed, owner, repo, search: query || null, view },
  })
  const history = appendHistoryPage(
    { entries: snapshot.value.history, next_cursor: before },
    page,
    before,
  )
  repositorySearchResource.writeIfUnchanged(identity, snapshot, {
    ...snapshot.value,
    history: history.entries,
    historyCursor: history.next_cursor,
  }, snapshot.version ?? '')
}
