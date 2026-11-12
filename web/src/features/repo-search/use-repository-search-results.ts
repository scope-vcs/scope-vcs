import type { RepoParams } from '@/api/types'
import type { HistoryFeed, RepoSummaryResponse, ViewId } from '@/api/types.generated'
import { useCachedResource } from '@/lib/use-cached-resource'
import { loadHistoryPage } from '@/routes/-repo-history-actions'
import { loadRequestQueuePage } from '@/routes/-request-workspace-actions'
import { useCallback, useMemo } from 'react'
import { appendHistoryPage } from '../history/history-pagination'
import { historyFeedResource } from '../history/history-resource-cache'
import { REQUEST_QUEUE_SECTION_ORDER } from '../requests/request-list-model'
import {
  loadMoreRequestQueue,
  requestQueueIdentity,
  type LoadRequestQueuePage,
} from '../requests/request-queue-cache'
import { useRequestQueue } from '../requests/use-request-queue'
import type { RepositorySearchResults } from './repository-search-model'

export function useRepositorySearchResults({
  enabled,
  feed,
  params: { owner, repo: repoName },
  query,
  repo,
  scope,
  view,
}: {
  enabled: boolean
  feed: HistoryFeed
  params: RepoParams
  query: string
  repo: RepoSummaryResponse
  scope: string | null
  view: ViewId
}) {
  const search = query === '' ? null : query
  const sections = useMemo(
    () => REQUEST_QUEUE_SECTION_ORDER.filter((section) => search !== null || section !== 'done'),
    [search],
  )
  const queueIdentity = scope
    ? search ? JSON.stringify([scope, view, search]) : requestQueueIdentity(scope, view)
    : null
  const loadQueuePage = useCallback<LoadRequestQueuePage>(
    (section, cursor, signal) => loadRequestQueuePage({
      data: { cursor, owner, repo: repoName, search, section, view },
      signal,
    }),
    [owner, repoName, search, view],
  )
  const queue = useRequestQueue(queueIdentity, String(repo.change_version), loadQueuePage, enabled)

  const historyIdentity = scope ? [scope, view, feed, ...search ? [search] : []].join('\0') : null
  const loadHistory = useCallback(
    (before: string | null, signal?: AbortSignal) => loadHistoryPage({
      data: { before, feed, owner, repo: repoName, search, view },
      signal,
    }),
    [feed, owner, repoName, search, view],
  )
  const history = useCachedResource({
    enabled,
    fallbackError: 'History is unavailable.',
    identity: historyIdentity,
    load: useCallback(
      (signal: AbortSignal) => loadHistory(null, signal)
        .then(({ entries, next_cursor }) => ({ entries, next_cursor })),
      [loadHistory],
    ),
    resource: historyFeedResource,
    version: String(repo.content_version),
  })

  const pages = queue.value?.pages
  const results = useMemo<RepositorySearchResults>(() => ({
    history: history.value?.entries ?? [],
    requests: sections.flatMap((section) => pages?.[section].requests ?? []),
  }), [history.value, pages, sections])
  const pagedSections = sections.filter((section) => pages?.[section].next_cursor)

  async function loadMoreRequests() {
    if (!queueIdentity) return
    for (const section of pagedSections) await loadMoreRequestQueue(queueIdentity, section, loadQueuePage)
  }

  async function loadOlderHistory() {
    if (!historyIdentity) return
    const snapshot = historyFeedResource.getSnapshot(historyIdentity)
    const before = snapshot.value?.next_cursor
    if (!snapshot.value || !before) return
    const page = await loadHistory(before)
    historyFeedResource.writeIfUnchanged(
      historyIdentity,
      snapshot,
      appendHistoryPage(snapshot.value, page, before),
      snapshot.version ?? '',
    )
  }

  return {
    failure: queue.error ? queue : history.error ? history : null,
    hasMoreHistory: Boolean(history.value?.next_cursor),
    hasMoreRequests: pagedSections.length > 0,
    loadMoreRequests,
    loadOlderHistory,
    loaded: queue.status === 'loaded' && history.status === 'loaded',
    results,
  }
}
