import type { RepoParams } from '@/api/types'
import type { RepoSummaryResponse } from '@/api/types.generated'
import { DiffStatSummary } from '@/components/diff-stat'
import { RelativeTimestamp } from '@/components/timestamp'
import { Badge } from '@/components/ui/badge'
import { historyEntryCacheKey, historyEntryResource } from '@/features/history/history-resource-cache'
import { reloadRequestStateValue, requestStateIdentity, requestStateResource } from '@/features/requests/request-state-resource'
import { requestMergeabilityLabel, requestMergeabilityTone } from '@/features/requests/request-labels'
import { diffStatWithChanges } from '@/lib/diff-stat'
import { useCachedResource } from '@/lib/use-cached-resource'
import { cn } from '@/lib/utils'
import { loadHistoryEntry } from '@/routes/-repo-history-actions'
import { loadRequestChildrenForViewer, loadRequestStateForViewer } from '@/routes/-request-state-actions'
import { useAuth } from '@clerk/tanstack-react-start'
import { Link } from '@tanstack/react-router'
import { useCallback, type ReactNode } from 'react'
import { repoResourceScope } from './repo-resource-scope'
import { useViewingAs } from './use-viewing-as'
import { viewingAsSearch } from './viewing-as'

const STRIP = 'flex min-h-11 flex-wrap items-center gap-x-5 gap-y-0.5 border-t px-(--app-inset) py-2 text-xs sm:flex-nowrap'
const LINK = 'shrink-0 rounded font-medium text-foreground underline underline-offset-4 focus-visible:outline-2 focus-visible:outline-ring'

export function RepositoryRequestStrip({
  params,
  repo,
  requestId,
}: {
  params: RepoParams
  repo: RepoSummaryResponse
  requestId: string
}) {
  const { isLoaded, userId } = useAuth()
  const viewerId = userId ?? null
  const scope = isLoaded ? repoResourceScope(repo, viewerId) : null
  const identity = scope ? requestStateIdentity(scope, requestId) : null
  const load = useCallback((signal: AbortSignal) => {
    if (identity === null) throw new Error('Request state scope is unavailable.')
    const request = { ...params, request_id: requestId }
    return reloadRequestStateValue(
      identity,
      (stateSignal) => loadRequestStateForViewer(request, viewerId, stateSignal),
      (cursor, childSignal) => loadRequestChildrenForViewer({ ...request, cursor }, viewerId, childSignal),
      signal,
    )
  }, [identity, params, requestId, viewerId])
  const current = useCachedResource({
    fallbackError: 'Request details are unavailable.',
    identity,
    load,
    resource: requestStateResource,
  })
  const detail = current.value?.state?.detail
  const changes = diffStatWithChanges(detail?.diff_stat ?? null)
  return (
    <VersionStrip
      className="border-info-border bg-info-soft text-info-strong"
      title={detail?.request.title ?? 'Request'}
    >
      {detail && (
        <>
          <Badge variant={requestMergeabilityTone(detail.request)}>{requestMergeabilityLabel(detail.request)}</Badge>
          {changes && <DiffStatSummary detail="exact" stat={changes} />}
          <span className="font-mono" title={detail.request.head_oid}>{detail.request.head_oid.slice(0, 7)}</span>
        </>
      )}
      <Link className={LINK} params={{ ...params, requestId }} search={{}} to="/$owner/$repo/requests/$requestId">
        Open request
      </Link>
    </VersionStrip>
  )
}

export function RepositoryUpdateStrip({
  entryId,
  params,
  repo,
  selectedPath,
}: {
  entryId: string
  params: RepoParams
  repo: RepoSummaryResponse
  selectedPath: string | null
}) {
  const { isLoaded, userId } = useAuth()
  const { reader, view } = useViewingAs()
  const scope = isLoaded ? repoResourceScope(repo, userId ?? null) : null
  const load = useCallback(async (signal: AbortSignal) => {
    const { entry } = await loadHistoryEntry({
      data: { entry: entryId, owner: params.owner, repo: params.repo, view },
      signal,
    })
    return entry
  }, [entryId, params.owner, params.repo, view])
  const current = useCachedResource({
    fallbackError: 'Update details are unavailable.',
    identity: scope ? historyEntryCacheKey({ entry: entryId, scope, view }) : null,
    load,
    resource: historyEntryResource,
  })
  const entry = current.value
  return (
    <VersionStrip
      className="border-warning-border bg-warning-soft text-warning-strong"
      title={pastUpdateTitle(entry?.message ?? '')}
    >
      <span className="shrink-0 rounded-full border border-warning-border bg-card px-2">Past update</span>
      {entry?.author && <span className="min-w-0 max-w-40 truncate">{entry.author}</span>}
      {entry && entry.occurred_at_unix !== null && <RelativeTimestamp className="shrink-0" value={entry.occurred_at_unix} />}
      <Link
        className={LINK}
        params={{ ...params, entryId }}
        search={viewingAsSearch(view, reader)}
        to="/$owner/$repo/updates/$entryId"
      >
        View changes
      </Link>
      <Link
        className={LINK}
        params={params}
        search={{ file: selectedPath ?? undefined }}
        to="/$owner/$repo/code"
      >
        Back to latest
      </Link>
    </VersionStrip>
  )
}

function VersionStrip({
  children,
  className,
  title,
}: {
  children: ReactNode
  className: string
  title: string
}) {
  return (
    <div className={cn(STRIP, className)} data-repo-activity>
      <p className="min-w-0 basis-full truncate text-sm font-medium text-foreground sm:flex-1 sm:basis-auto" title={title}>
        {title}
      </p>
      {children}
    </div>
  )
}

function pastUpdateTitle(message: string) {
  const [firstLine = ''] = message.split('\n', 1)
  return firstLine === '' ? 'Past update' : firstLine
}
