import { Badge } from '@/components/ui/badge'
import { Button } from '@/components/ui/button'
import { BlockSkeleton, TextSkeleton, type TextSkeletonLength } from '@/components/ui/skeleton'
import { historyEntryLabels } from '@/features/history/history-row-labels'
import { cn } from '@/lib/utils'
import { History, LoaderCircle } from 'lucide-react'
import type { HistoryEntrySummaryResponse } from '@/api/types.generated'

const HISTORY_ENTRY_ROW_CLASS =
  'grid w-full grid-cols-[minmax(0,1fr)_auto] items-center gap-x-3 px-5 py-2.5 text-left text-sm sm:px-6 lg:px-8'

const PENDING_TITLE_LENGTHS: TextSkeletonLength[] = ['long', 'medium', 'long', 'short', 'medium']

/** Enough rows to fill the list box the way an active repository's history does. */
export function HistoryEntryListSkeleton() {
  return (
    <div className="py-2">
      {PENDING_TITLE_LENGTHS.map((length, row) => (
        <div className={HISTORY_ENTRY_ROW_CLASS} key={`entry-${row}`}>
          <span className="flex min-w-0 items-center gap-2">
            <BlockSkeleton className="size-3.5 shrink-0" />
            <span className="min-w-0">
              <span className="flex min-w-0 items-center gap-2">
                <BlockSkeleton className="h-5 w-11 shrink-0 rounded-md" />
                <TextSkeleton length={length} />
              </span>
              <TextSkeleton className="mt-0.5" length="short" size="meta" />
            </span>
          </span>
          <TextSkeleton length="short" size="meta" />
        </div>
      ))}
    </div>
  )
}

export function HistoryEntryList({
  entries,
  loadOlderError,
  loadingOlder,
  onLoadOlder,
  onSelectEntry,
  selectedEntryId,
  showLoadOlder,
}: {
  entries: HistoryEntrySummaryResponse[]
  loadOlderError: string | null
  loadingOlder: boolean
  onLoadOlder: () => void
  onSelectEntry: (entry: HistoryEntrySummaryResponse) => void
  selectedEntryId: string | null
  showLoadOlder: boolean
}) {
  return (
    <div className="py-2">
      {entries.map((entry) => {
        const labels = historyEntryLabels(entry)
        const selected = selectedEntryId === entry.source_id
        return (
          <button
            aria-label={labels.ariaLabel}
            aria-pressed={selected}
            className={cn(
              HISTORY_ENTRY_ROW_CLASS,
              'transition-colors',
              selected
                ? 'bg-accent shadow-[inset_2px_0_0_0_var(--foreground)]'
                : 'hover:bg-accent/50',
            )}
            key={entry.id}
            onClick={() => onSelectEntry(entry)}
            title={entry.source_id}
            type="button"
          >
            <span className="flex min-w-0 items-center gap-2">
              <History className="size-3.5 shrink-0 text-muted-foreground" />
              <span className="min-w-0">
                <span className="flex min-w-0 items-center gap-2">
                  <Badge className="shrink-0" variant="neutral">{labels.kind}</Badge>
                  <span className="truncate text-[13px] font-medium">{labels.title}</span>
                </span>
                <span className="mt-0.5 block truncate font-mono text-[11px] leading-4 text-muted-foreground">
                  {labels.compactId}
                </span>
              </span>
            </span>
            <span className="max-w-28 text-right text-xs tabular-nums text-muted-foreground sm:max-w-56">
              {labels.count}
            </span>
          </button>
        )
      })}
      {showLoadOlder ? (
        <div className="flex flex-col items-center gap-2 px-5 py-4">
          <Button
            disabled={loadingOlder}
            onClick={onLoadOlder}
            size="sm"
            variant="secondary"
          >
            {loadingOlder ? <LoaderCircle className="animate-spin" /> : null}
            Load older history
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
