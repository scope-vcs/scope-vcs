import type { RepoParams } from '@/api/types'
import { PanelState } from '@/components/empty-state'
import { PendingSurface } from '@/components/pending-surface'
import { RelativeTimestamp } from '@/components/timestamp'
import { Button } from '@/components/ui/button'
import { TextSkeleton } from '@/components/ui/skeleton'
import { historyEntryLabels } from '@/features/history/history-row-labels'
import { Link } from '@tanstack/react-router'
import { LoaderCircle } from 'lucide-react'
import type { ReactNode } from 'react'
import type { HistoryEntrySummaryResponse } from '@/api/types.generated'
import type { useHistoryFeed } from './history-feed'
import type { UpdateSearch } from './update-search'

/** A history feed's loading, error, empty and loaded states. */
export function HistoryFeedList({
  empty,
  history: { loadOlder, loadOlderError, loadingOlder, resource },
  onNavigate,
  params,
  search,
}: {
  empty: string
  history: ReturnType<typeof useHistoryFeed>
  onNavigate?: () => void
  params: RepoParams
  search: UpdateSearch
}) {
  if (resource.status === 'failed') {
    return (
      <PanelState tone="error">
        <span>{resource.error}</span>
        <Button onClick={resource.retry} size="sm" variant="secondary">Retry</Button>
      </PanelState>
    )
  }
  if (resource.status !== 'loaded') {
    return (
      <PendingSurface label="Loading history" onRetry={resource.retry}>
        <div className="grid gap-4 p-3">
          <TextSkeleton length="long" />
          <TextSkeleton length="medium" />
          <TextSkeleton length="long" />
        </div>
      </PendingSurface>
    )
  }
  if (!resource.value.entries.length) {
    return <p className="px-3 py-6 text-center text-xs text-muted-foreground">{empty}</p>
  }
  return (
    <HistoryEntryList
      entries={resource.value.entries}
      loadOlderError={loadOlderError}
      loadingOlder={loadingOlder}
      onLoadOlder={() => void loadOlder()}
      onNavigate={onNavigate}
      params={params}
      search={search}
      showLoadOlder={resource.value.next_cursor !== null}
    />
  )
}

function HistoryEntryList({
  entries,
  loadOlderError,
  loadingOlder,
  onLoadOlder,
  onNavigate,
  params,
  search,
  showLoadOlder,
}: {
  entries: HistoryEntrySummaryResponse[]
  loadOlderError: string | null
  loadingOlder: boolean
  onLoadOlder: () => void
  onNavigate?: () => void
  params: RepoParams
  search: UpdateSearch
  showLoadOlder: boolean
}) {
  return (
    <div>
      <ul className="divide-y divide-border">
        {entries.map((entry) => {
          const labels = historyEntryLabels(entry)
          return (
            <li key={entry.id}>
              <Link
                className="grid grid-cols-[minmax(0,1fr)_auto] items-center gap-x-4 px-3 py-2.5 text-left hover:bg-muted focus-visible:bg-muted focus-visible:outline-2 focus-visible:-outline-offset-2 focus-visible:outline-ring"
                onClick={onNavigate}
                params={{ ...params, entryId: entry.source_id }}
                search={search}
                title={entry.message}
                to="/$owner/$repo/updates/$entryId"
              >
                <span className="min-w-0">
                  <span className="block truncate text-[13px] font-medium">{labels.title}</span>
                  <span className="mt-0.5 flex min-w-0 items-center gap-1.5 text-xs text-muted-foreground">
                    {labels.kind ? <span className="shrink-0 font-medium text-foreground/80">{labels.kind}</span> : null}
                    {entry.author ? <MetaItem first={!labels.kind}>{entry.author}</MetaItem> : null}
                    {entry.occurred_at_unix !== null
                      ? <MetaItem first={!labels.kind && !entry.author}><RelativeTimestamp value={entry.occurred_at_unix} /></MetaItem>
                      : null}
                  </span>
                </span>
                <span className="max-w-40 text-right text-xs tabular-nums text-muted-foreground">
                  {labels.count}
                </span>
              </Link>
            </li>
          )
        })}
      </ul>
      {showLoadOlder ? (
        <div className="flex flex-col items-center gap-2 border-t border-border px-3 py-3">
          <Button
            disabled={loadingOlder}
            onClick={onLoadOlder}
            size="sm"
            variant="ghost"
          >
            {loadingOlder ? <LoaderCircle className="animate-spin" /> : null}
            Load older
          </Button>
          {loadOlderError ? (
            <span className="text-center text-xs text-destructive" role="alert">
              {loadOlderError}
            </span>
          ) : null}
        </div>
      ) : null}
    </div>
  )
}

function MetaItem({ children, first }: { children: ReactNode; first: boolean }) {
  return (
    <span className="flex min-w-0 items-center gap-1.5 truncate">
      {first ? null : <span aria-hidden="true">·</span>}
      {children}
    </span>
  )
}
