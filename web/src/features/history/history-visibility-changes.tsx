import { VisibilityBadge } from '@/components/visibility-badge'
import { ArrowRight } from 'lucide-react'
import type { HistoryEntryDetailResponse } from '@/api/types.generated'

export type HistoryVisibilityChange = HistoryEntryDetailResponse['visibility_changes'][number]

export function VisibilityChanges({
  changes,
  onSelect,
  selectedId,
}: {
  changes: HistoryEntryDetailResponse['visibility_changes']
  onSelect: (change: HistoryVisibilityChange) => void
  selectedId: string | null
}) {
  if (changes.length === 0) return null
  const rows = (
    <div className="divide-y divide-border">
      {changes.map((change) => (
        <div className="flex min-h-10 flex-wrap items-center gap-2 px-5 py-2 sm:px-6" key={change.id}>
          {change.file ? (
            <button
              aria-pressed={selectedId === change.id}
              className="min-w-0 flex-1 break-all text-left font-mono text-xs text-foreground underline-offset-4 hover:underline"
              onClick={() => onSelect(change)}
              type="button"
            >
              {change.path}
            </button>
          ) : (
            <span className="min-w-0 flex-1 break-all font-mono text-xs">{change.path}</span>
          )}
          <span className="text-xs text-muted-foreground">
            {change.new_visibility === 'Public' ? 'Made public' : 'Made private'}
          </span>
          <span className="flex items-center gap-2">
            <VisibilityBadge compact visibility={change.old_visibility} />
            <ArrowRight aria-hidden className="size-3 shrink-0 text-muted-foreground" />
            <VisibilityBadge compact visibility={change.new_visibility} />
          </span>
        </div>
      ))}
    </div>
  )
  return (
    <section aria-label="Visibility changes" className="border-b border-border">
      <h2 className="px-5 pt-3 pb-1 text-sm font-medium sm:px-6">Visibility changes</h2>
      {rows}
    </section>
  )
}
