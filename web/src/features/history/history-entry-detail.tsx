import type { HistoryEntryDetail } from '@/api/types'
import { PanelState } from '@/components/empty-state'
import { Badge } from '@/components/ui/badge'
import { Button } from '@/components/ui/button'
import { PendingSurface } from '@/components/pending-surface'
import type { CachedResource } from '@/lib/use-cached-resource'
import { ChangedFilesWorkbench, useChangedFileNavigation, type ChangedFilesProps } from './changed-files-workbench'
import { CommitDetailSkeleton } from './history-commit-detail-skeleton'
import { historyCommitTitle, historyEntryCountLabel, historyEntryKindLabel } from './history-row-labels'
import { VisibilityChanges, type HistoryVisibilityChange } from './history-visibility-changes'

export function HistoryEntryDetailPanel(props: ChangedFilesProps & {
  resource: CachedResource<HistoryEntryDetail>
  onRetryDetail: () => void
  onSelectVisibility: (change: HistoryVisibilityChange) => void
  selectedVisibilityId: string | null
}) {
  const { resource, onCloseDiff, onRetryDetail, onSelectVisibility, selectedFilePath, selectedVisibilityId } = props
  const navigation = useChangedFileNavigation(onCloseDiff)
  if (resource.status === 'loading') {
    return (
      <PendingSurface delay label="Loading update details">
        <CommitDetailSkeleton showDiff={selectedFilePath !== null} />
      </PendingSurface>
    )
  }
  if (resource.status === 'failed') {
    return (
      <PanelState tone="error">
        <span>{resource.error}</span>
        <Button onClick={onRetryDetail} size="sm" variant="secondary">Retry</Button>
      </PanelState>
    )
  }
  if (!resource.value) return <PanelState><span>Select an update</span></PanelState>
  const detail = resource.value
  return (
    <div className="scope-content-enter min-w-0">
      <div className="border-b border-border px-5 py-4 sm:px-6">
        <div className="flex items-start gap-2">
          <Badge variant="neutral">{historyEntryKindLabel(detail.kind)}</Badge>
          <h3 className="min-w-0 break-words text-sm font-semibold leading-5">{historyCommitTitle(detail)}</h3>
        </div>
        <div className="mt-1.5 flex flex-wrap gap-x-2 gap-y-1 break-all font-mono text-xs text-muted-foreground">
          <span>{detail.source_id}</span>
          {detail.author ? <span>· {detail.author}</span> : null}
        </div>
        <p className="mt-2 text-xs text-muted-foreground">{historyEntryCountLabel(detail)}</p>
      </div>
      <VisibilityChanges
        changes={detail.visibility_changes}
        expanded={detail.kind === 'visibility_change'}
        onSelect={onSelectVisibility}
        selectedId={selectedVisibilityId}
      />
      <ChangedFilesWorkbench
        {...props}
        files={detail.files}
        navigation={navigation}
        navigationLabel="Update file navigator"
      />
    </div>
  )
}
