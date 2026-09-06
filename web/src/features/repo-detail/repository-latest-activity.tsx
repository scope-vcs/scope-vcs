import type { RepoParams, RepoSummary } from '@/api/types'
import { PendingSurface } from '@/components/pending-surface'
import { RelativeTimestamp } from '@/components/timestamp'
import { TextSkeleton } from '@/components/ui/skeleton'
import { useCachedResource } from '@/lib/use-cached-resource'
import { useAuth } from '@clerk/tanstack-react-start'
import { loadRepositoryLatestActivity } from '@/routes/-repo-activity-actions'
import { Link } from '@tanstack/react-router'
import { History } from 'lucide-react'
import { useCallback } from 'react'
import { repoResourceScope } from './repo-resource-scope'
import { repositoryActivityResource } from './repository-activity-resource'

export function RepositoryLatestActivity({ params, repo }: { params: RepoParams; repo: RepoSummary }) {
  const { isLoaded, userId } = useAuth()
  const ready = repo.lifecycle_state === 'Ready'
  const identity = ready && isLoaded ? repoResourceScope(repo, userId ?? null) : null
  const load = useCallback((signal: AbortSignal) => loadRepositoryLatestActivity({
    data: { owner: params.owner, repo: params.repo }, signal,
  }), [params.owner, params.repo])
  const current = useCachedResource({
    identity,
    load,
    resource: repositoryActivityResource,
    version: String(repo.change_version),
    fallbackError: 'Latest change unavailable.',
  })

  if (!ready) return null
  if (current.status === 'loading' || current.status === 'idle') {
    return (
      <div className="border-b border-border px-5 py-3 sm:px-6 lg:px-8">
        <PendingSurface delay label="Loading latest repository change">
          <TextSkeleton length="long" size="meta" />
        </PendingSurface>
      </div>
    )
  }
  if (current.status === 'failed') {
    return (
      <div className="flex items-center gap-3 border-b border-border px-5 py-3 text-xs text-muted-foreground sm:px-6 lg:px-8">
        <output>Latest change unavailable.</output>
        <button className="rounded underline underline-offset-4 focus-visible:outline-2 focus-visible:outline-ring" onClick={current.retry} type="button">
          Retry latest change
        </button>
      </div>
    )
  }
  const { audience, entry, head_oid: headOid } = current.value
  if (!entry) return null
  const message = entry.message.split('\n', 1)[0] || 'Repository updated'
  return (
    <div aria-label="Latest repository change" className="flex flex-wrap items-center gap-x-4 gap-y-2 border-b border-border px-5 py-3 text-xs sm:px-6 lg:px-8">
      <Link
        className="min-w-0 basis-full truncate rounded font-medium hover:underline focus-visible:outline-2 focus-visible:outline-ring sm:flex-1 sm:basis-auto"
        params={params}
        search={{ audience, entry: entry.source_id, feed: 'all' }}
        title={message}
        to="/$owner/$repo/history"
      >
        {message}
      </Link>
      <div className="flex min-w-0 flex-1 flex-wrap items-center gap-x-3 gap-y-1 text-muted-foreground sm:flex-none">
        {entry.author && <span className="max-w-40 truncate" title={entry.author}>{entry.author}</span>}
        {entry.occurred_at_unix !== null && <RelativeTimestamp value={entry.occurred_at_unix} />}
        {headOid && (
          <span className="font-mono text-[11px]" title={`Current revision: ${headOid}`}>
            <span className="sr-only">Current revision </span>{headOid.slice(0, 7)}
          </span>
        )}
      </div>
      <Link className="flex shrink-0 items-center gap-1.5 rounded text-muted-foreground hover:text-foreground focus-visible:outline-2 focus-visible:outline-ring" params={params} search={{ audience, feed: 'all' }} to="/$owner/$repo/history">
        <History aria-hidden="true" className="size-3.5" /> History
      </Link>
      {current.error && <button className="basis-full text-left underline" onClick={current.retry} type="button">Could not refresh latest change. Retry</button>}
    </div>
  )
}
