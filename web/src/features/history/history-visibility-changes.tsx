import type { HistoryEntryDetail } from '@/api/types'
import { VisibilityBadge } from '@/components/visibility-badge'
import { ArrowRight } from 'lucide-react'

export type HistoryVisibilityChange = HistoryEntryDetail['visibility_changes'][number]

export function VisibilityChanges({
  changes,
  expanded,
  onSelect,
  selectedId,
}: {
  changes: HistoryEntryDetail['visibility_changes']
  expanded: boolean
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
              className="min-w-0 flex-1 break-all text-left font-mono text-xs text-brand underline-offset-4 hover:underline"
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
  return expanded ? (
    <section aria-label="Visibility changes" className="border-b border-border">
      <h4 className="px-5 py-3 text-sm font-medium sm:px-6">Visibility changes · {changes.length}</h4>
      {rows}
    </section>
  ) : (
    <details className="border-b border-border" open={selectedId ? true : undefined}>
      <summary className="cursor-pointer px-5 py-3 text-sm font-medium sm:px-6">
        Visibility changes · {changes.length}
      </summary>
      {rows}
    </details>
  )
}
