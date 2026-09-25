import type { RepoParams } from '@/api/types'
import type { HistoryFeed, ProjectionPreviewAudience } from '@/api/types.generated'
import { useRepoLayout } from '@/features/repo-detail/repo-layout-context'
import { repoResourceScope } from '@/features/repo-detail/repo-resource-scope'
import { createCachedResource } from '@/lib/cached-resource'
import { resourceErrorMessage, useCachedResource } from '@/lib/use-cached-resource'
import { loadHistoryPage } from '@/routes/-repo-history-actions'
import { useAuth } from '@clerk/tanstack-react-start'
import { useCallback, useState } from 'react'
import { appendHistoryPage, type LoadedHistory } from './history-pagination'

// One owner for every history list, so the dropdown and Settings share loaded
// pages and reopening either shows them without a refetch.
export const historyFeedResource = createCachedResource<LoadedHistory>({
  maxEntries: 16,
  maxWeight: 2 * 1024 * 1024,
  weightOf: (value) => JSON.stringify(value).length * 2,
})

export function useHistoryFeed({
  audience,
  enabled = true,
  feed,
  params,
}: {
  audience: ProjectionPreviewAudience
  enabled?: boolean
  feed: HistoryFeed
  params: RepoParams
}) {
  const { isLoaded, userId } = useAuth()
  const { repo } = useRepoLayout()
  const identity = isLoaded
    ? [repoResourceScope(repo, userId ?? null), audience, feed].join('\0')
    : null
  // A new change version restarts from the first page; older cursors expire with it.
  const version = String(repo.change_version)
  const { owner, repo: repoName } = params
  const load = useCallback(
    (signal: AbortSignal) => loadHistoryPage({
      data: { audience, before: null, feed, owner, repo: repoName },
      signal,
    }).then(({ entries, next_cursor }) => ({ entries, next_cursor })),
    [audience, feed, owner, repoName],
  )
  const resource = useCachedResource({
    enabled,
    fallbackError: 'History is unavailable.',
    identity,
    load,
    resource: historyFeedResource,
    version,
  })
  const [older, setOlder] = useState<{ identity: string | null; error: string | null; loading: boolean }>(
    { identity: null, error: null, loading: false },
  )
  const olderState = older.identity === identity ? older : { error: null, loading: false }

  const loadOlder = useCallback(async () => {
    if (!identity) return
    const snapshot = historyFeedResource.getSnapshot(identity)
    const before = snapshot.value?.next_cursor
    if (!snapshot.value || !before) return
    setOlder({ identity, error: null, loading: true })
    try {
      const page = await loadHistoryPage({
        data: { audience, before, feed, owner, repo: repoName },
      })
      historyFeedResource.writeIfUnchanged(
        identity,
        snapshot,
        appendHistoryPage(snapshot.value, page, before),
        snapshot.version ?? version,
      )
      setOlder({ identity, error: null, loading: false })
    } catch (error) {
      setOlder({ identity, error: resourceErrorMessage(error, 'Older history is unavailable.'), loading: false })
    }
  }, [audience, feed, identity, owner, repoName, version])

  return {
    loadOlder,
    loadOlderError: olderState.error,
    loadingOlder: olderState.loading,
    resource,
  }
}

export function defaultHistoryAudience(canReadPrivateFiles: boolean): ProjectionPreviewAudience {
  return canReadPrivateFiles ? 'private' : 'public'
}
