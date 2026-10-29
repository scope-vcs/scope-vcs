import { ToggleGroup, ToggleGroupItem } from '@/components/ui/toggle-group'
import { VisibilityBadge } from '@/components/visibility-badge'
import { useRepoViews } from '@/features/repo-detail/repo-layout-context'
import { cn } from '@/lib/utils'
import { ArrowRight } from 'lucide-react'
import type { RepoViews } from '@/api/repo-views'
import type { ViewId } from '@/api/types.generated'
import { changedPathTargets, type ChangedPath, type HistoryVisibilityChange } from './changed-paths-model'
import { transitionPathLabel } from './views-transition-model'

const CODE_TARGET = 'code'

export function ChangedPathContext({
  onSelectFile,
  onSelectVisibility,
  path,
  selectedVisibilityId,
  view,
}: {
  onSelectFile: (path: string) => void
  onSelectVisibility: (change: HistoryVisibilityChange) => void
  path: ChangedPath
  selectedVisibilityId: string | null
  view: ViewId
}) {
  const views = useRepoViews()
  if (path.visibility.length === 0) return null
  const targets = changedPathTargets(path)
  const selected = path.visibility.find((change) => change.id === selectedVisibilityId)?.id ?? CODE_TARGET
  return (
    <div className="grid gap-2 border-b border-border px-3 py-2">
      <VisibilityTransitions changes={path.visibility} view={view} />
      {targets.length > 1 && (
        <ToggleGroup
          aria-label="Diff to show"
          className="flex-wrap"
          onValueChange={(value) => {
            const target = targets.find((target) =>
              (target.kind === 'file' ? CODE_TARGET : target.change.id) === value)
            if (target?.kind === 'file') onSelectFile(target.file.path)
            else if (target) onSelectVisibility(target.change)
          }}
          type="single"
          value={selected}
        >
          {targets.map((target) => target.kind === 'file' ? (
            <ToggleGroupItem className="text-xs" key={CODE_TARGET} value={CODE_TARGET}>Code change</ToggleGroupItem>
          ) : (
            <ToggleGroupItem className="text-xs" key={target.change.id} value={target.change.id}>
              {transitionLabel(target.change, views, view)} preview
            </ToggleGroupItem>
          ))}
        </ToggleGroup>
      )}
    </div>
  )
}

export function VisibilityTransitions({
  changes,
  className,
  view,
}: {
  changes: HistoryVisibilityChange[]
  className?: string
  view: ViewId
}) {
  const views = useRepoViews()
  return (
    <div className={cn('grid gap-2', className)}>
      {changes.map((change) => (
        <div className="flex flex-wrap items-center gap-1.5 text-xs text-muted-foreground" key={change.id}>
          <VisibilityBadge compact visibility={change.old_label} />
          {change.old_label !== change.new_label && (
            <>
              <ArrowRight aria-hidden className="size-3 shrink-0" />
              <VisibilityBadge compact visibility={change.new_label} />
            </>
          )}
          <span className="text-foreground">{transitionLabel(change, views, view)}</span>
          {change.old_label !== change.new_label && <span>· was {views.name(change.old_label)}</span>}
        </div>
      ))}
    </div>
  )
}

function transitionLabel(change: HistoryVisibilityChange, views: RepoViews, view: ViewId) {
  return transitionPathLabel(change, views.name(view)) ?? `Moved to ${views.name(change.new_label)}`
}
