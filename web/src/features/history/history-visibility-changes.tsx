import { VisibilityBadge } from '@/components/visibility-badge'
import { useRepoViews } from '@/features/repo-detail/repo-layout-context'
import { ArrowRight } from 'lucide-react'
import type { HistoryEntryDetailResponse, ViewId, ViewsTransition } from '@/api/types.generated'
import { transitionPathLabel, viewsTransitionChanges } from './views-transition-model'

export type HistoryVisibilityChange = HistoryEntryDetailResponse['visibility_changes'][number]

export function ViewsChanges({ transition }: { transition: ViewsTransition | null }) {
  const changes = transition ? viewsTransitionChanges(transition) : []
  if (changes.length === 0) return null
  return (
    <section aria-label="Views changes" className="border-b border-border">
      <h2 className="px-5 pt-3 pb-1 text-sm font-medium sm:px-6">Views changes</h2>
      <ul className="divide-y divide-border">
        {changes.map((change) => (
          <li className="px-5 py-2 text-xs sm:px-6" key={change}>{change}</li>
        ))}
      </ul>
    </section>
  )
}

export function VisibilityChanges({
  changes,
  onSelect,
  selectedId,
  view,
}: {
  changes: HistoryEntryDetailResponse['visibility_changes']
  onSelect: (change: HistoryVisibilityChange) => void
  selectedId: string | null
  view: ViewId
}) {
  const views = useRepoViews()
  if (changes.length === 0) return null
  const rows = (
    <div className="divide-y divide-border">
      {changes.map((change) => (
        <div className="flex min-h-10 flex-wrap items-center gap-2 px-5 py-2 sm:px-6" key={change.id}>
          {change.file ? (
            <button
              aria-pressed={selectedId === change.id}
              className="min-w-0 flex-[1_1_8rem] break-all text-left font-mono text-xs text-foreground underline-offset-4 hover:underline"
              onClick={() => onSelect(change)}
              type="button"
            >
              {change.path}
            </button>
          ) : (
            <span className="min-w-0 flex-[1_1_8rem] break-all font-mono text-xs">{change.path}</span>
          )}
          <span className="min-w-0 break-words text-xs text-muted-foreground">
            {transitionPathLabel(change, views.name(view)) ?? `Moved to ${views.name(change.new_label)}`}
          </span>
          <span className="flex items-center gap-2">
            <VisibilityBadge compact visibility={change.old_label} />
            {change.old_label !== change.new_label && (
              <>
                <ArrowRight aria-hidden className="size-3 shrink-0 text-muted-foreground" />
                <VisibilityBadge compact visibility={change.new_label} />
              </>
            )}
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
