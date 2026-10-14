import type { RepoParams } from '@/api/types'
import type { RepoSummaryResponse } from '@/api/types.generated'
import { PendingSurface } from '@/components/pending-surface'
import { RelativeTimestamp } from '@/components/timestamp'
import { TextSkeleton } from '@/components/ui/text-skeleton'
import { useCachedResource } from '@/lib/use-cached-resource'
import { useAuth } from '@clerk/tanstack-react-start'
import { loadRepositoryLatestActivity } from '@/routes/-repo-activity-actions'
import { Link } from '@tanstack/react-router'
import { useCallback } from 'react'
import { cn } from '@/lib/utils'
import { repoResourceScope } from './repo-resource-scope'
import { repositoryActivityIdentity, repositoryActivityResource } from './repository-activity-resource'
import { HistoryMenu } from '@/features/history/history-menu'
import { useViewingAs } from './use-viewing-as'
import { viewingAsSearch } from './viewing-as'

export function RepositoryLatestActivity({ params, repo }: { params: RepoParams; repo: RepoSummaryResponse }) {
  const { isLoaded, userId } = useAuth()
  const ready = repo.lifecycle_state === 'Ready'
  const { reader, view: selectedView } = useViewingAs()
  const identity = ready && isLoaded
    ? repositoryActivityIdentity(repoResourceScope(repo, userId ?? null), selectedView)
    : null
  const load = useCallback((signal: AbortSignal) => loadRepositoryLatestActivity({
    data: { owner: params.owner, repo: params.repo, view: selectedView }, signal,
  }), [params.owner, params.repo, selectedView])
  const current = useCachedResource({
    identity,
    load,
    resource: repositoryActivityResource,
    version: String(repo.content_version),
    fallbackError: 'Latest change unavailable.',
  })

  if (!ready) return null
  if (current.status === 'loading' || current.status === 'idle') {
    return <RepositoryLatestActivityPending onRetry={current.retry} />
  }
  if (current.status === 'failed') {
    return (
      <div className="flex items-center gap-3 text-xs text-muted-foreground">
        <output>Latest change unavailable.</output>
        <button className="rounded underline underline-offset-4 focus-visible:outline-2 focus-visible:outline-ring" onClick={current.retry} type="button">
          Retry latest change
        </button>
      </div>
    )
  }
  const { entry, head_oid: headOid } = current.value
  if (!entry) return null
  const message = entry.message.split('\n', 1)[0] || 'Repository updated'
  return (
    <div
      aria-label="Latest repository change"
      className={cn('flex min-w-0 flex-wrap items-center gap-x-3 gap-y-1 text-xs', !current.error && 'sm:flex-nowrap')}
    >
      <Link
        className="min-w-0 basis-full truncate rounded text-sm font-medium text-foreground hover:underline focus-visible:outline-2 focus-visible:outline-ring sm:basis-auto"
        params={{ ...params, entryId: entry.source_id }}
        search={viewingAsSearch(selectedView, reader)}
        title={message}
        to="/$owner/$repo/updates/$entryId"
      >
        {message}
      </Link>
      <div className="flex min-w-0 flex-wrap items-center gap-x-2 text-muted-foreground">
        {entry.author && <span className="max-w-40 truncate" title={entry.author}>{entry.author}</span>}
        {entry.occurred_at_unix !== null && <RelativeTimestamp value={entry.occurred_at_unix} />}
        {headOid && (
          <span className="font-mono text-xs" title={`Current revision: ${headOid}`}>
            <span className="sr-only">Current revision </span>{headOid.slice(0, 7)}
          </span>
        )}
      </div>
      <div className="ml-auto sm:ml-0">
        <HistoryMenu params={params} />
      </div>
      {current.error && <button className="basis-full text-left underline" onClick={current.retry} type="button">Could not refresh latest change. Retry</button>}
    </div>
  )
}

export function RepositoryLatestActivityPending({ onRetry }: { onRetry?: () => void }) {
  return (
    <PendingSurface label="Loading latest repository change" onRetry={onRetry}>
      <div className="flex flex-wrap items-center gap-x-3 gap-y-1 sm:flex-nowrap">
        <span className="flex h-5 basis-full items-center sm:basis-auto">
          <TextSkeleton length="long" size="meta" />
        </span>
        <span className="flex h-4 items-center">
          <TextSkeleton length="short" size="meta" />
        </span>
      </div>
    </PendingSurface>
  )
}
