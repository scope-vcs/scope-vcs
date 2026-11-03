import type { RepoParams } from '@/api/types'
import type { HistoryFeed, ViewId } from '@/api/types.generated'
import { loadHistoryPage } from '@/routes/-repo-history-actions'
import { loadRequestQueuePage } from '@/routes/-request-workspace-actions'
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
    requests: pages.flatMap((page) => page.requests),
  }
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
  repositorySearchResource.writeIfUnchanged(identity, snapshot, {
    ...snapshot.value,
    history: [...snapshot.value.history, ...page.entries],
    historyCursor: page.next_cursor,
  }, snapshot.version ?? '')
}
