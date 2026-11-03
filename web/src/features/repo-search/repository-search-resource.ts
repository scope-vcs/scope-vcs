import type { RepoParams } from '@/api/types'
import type { ViewId } from '@/api/types.generated'
import { loadHistoryPage } from '@/routes/-repo-history-actions'
import { loadRequestQueuePage } from '@/routes/-request-workspace-actions'
import { REQUEST_QUEUE_SECTION_ORDER } from '../requests/request-list-model'
import type { RepositorySearchResults } from './repository-search-model'

export async function loadRepositorySearch(
  { owner, repo }: RepoParams,
  view: ViewId,
  query: string,
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
      data: { before: null, feed: 'all', owner, repo, search, view },
      signal,
    }),
  ])
  return { history: history.entries, requests: pages.flatMap((page) => page.requests) }
}
