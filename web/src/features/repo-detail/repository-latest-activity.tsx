import type { HistoryEntrySummary, ProjectionPreviewAudience, RepoParams, RepoSummary } from '@/api/types'
import { PendingSurface } from '@/components/pending-surface'
import { RelativeTimestamp } from '@/components/timestamp'
import { TextSkeleton } from '@/components/ui/skeleton'
import { startAbortableResourceAttempt } from '@/lib/use-cached-resource'
import { loadRepositoryLatestActivity } from '@/routes/-repo-activity-actions'
import { Link } from '@tanstack/react-router'
import { History } from 'lucide-react'
import { useCallback, useEffect, useReducer, useState } from 'react'
import { useRepoChangeSubscription } from './repo-layout-context'

type Activity = {
  audience: ProjectionPreviewAudience
  entry: HistoryEntrySummary | null
  head_oid: string | null
}
type ActivityState = {
  identity: string
} & (
  | { status: 'loaded'; value: Activity }
  | { status: 'failed' }
)

export function RepositoryLatestActivity({ params, repo }: { params: RepoParams; repo: RepoSummary }) {
  const [refresh, retry] = useReducer((version: number) => version + 1, 0)
  const [state, setState] = useState<ActivityState | null>(null)
  const identity = [repo.id, repo.access.can_read_private_files, repo.change_version].join('\0')
  const ready = repo.lifecycle_state === 'Ready'

  useRepoChangeSubscription(useCallback((event) => {
    if (event.repo_id !== repo.id) return
    if (event.kind === 'Lagged' || typeof event.kind === 'object' && 'RepositoryChanged' in event.kind) {
      retry()
    }
  }, [repo.id]))

  useEffect(() => {
    if (!ready) return
    return startAbortableResourceAttempt({
      load: (signal) => loadRepositoryLatestActivity({ data: { owner: params.owner, repo: params.repo }, signal }),
      onLoaded: (value) => setState({ identity, status: 'loaded', value }),
      onFailed: () => setState({ identity, status: 'failed' }),
    })
  }, [identity, params.owner, params.repo, ready, refresh])

  if (!ready) return null
  const current = state?.identity === identity ? state : null
  if (!current) {
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
        <button className="rounded underline underline-offset-4 focus-visible:outline-2 focus-visible:outline-ring" onClick={retry} type="button">
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
    </div>
  )
}
