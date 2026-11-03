import type { RepoParams } from '@/api/types'
import type { HistoryFeed, ViewId } from '@/api/types.generated'
import { useRepoLayout } from '@/features/repo-detail/repo-layout-context'
import { repoResourceScope } from '@/features/repo-detail/repo-resource-scope'
import { useCachedResource } from '@/lib/use-cached-resource'
import { loadHistoryPage } from '@/routes/-repo-history-actions'
import { useAuth } from '@clerk/tanstack-react-start'
import { useCallback } from 'react'
import { historyFeedResource } from './history-resource-cache'

export function useHistoryFeed({
  view,
  enabled = true,
  feed,
  params,
}: {
  view: ViewId
  enabled?: boolean
  feed: HistoryFeed
  params: RepoParams
}) {
  const { isLoaded, userId } = useAuth()
  const { repo } = useRepoLayout()
  const identity = isLoaded
    ? [repoResourceScope(repo, userId ?? null), view, feed].join('\0')
    : null
  const version = String(repo.content_version)
  const { owner, repo: repoName } = params
  const load = useCallback(
    (signal: AbortSignal) => loadHistoryPage({
      data: { view, before: null, feed, owner, repo: repoName },
      signal,
    }).then(({ entries, next_cursor }) => ({ entries, next_cursor })),
    [view, feed, owner, repoName],
  )
  const resource = useCachedResource({
    enabled,
    fallbackError: 'History is unavailable.',
    identity,
    load,
    resource: historyFeedResource,
    version,
  })
  return { resource, view }
}
