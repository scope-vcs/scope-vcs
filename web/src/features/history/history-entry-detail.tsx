import { PanelState } from '@/components/empty-state'
import { Button } from '@/components/ui/button'
import { PendingSurface } from '@/components/pending-surface'
import { AbsoluteTimestamp } from '@/components/timestamp'
import type { CachedResource } from '@/lib/use-cached-resource'
import { ChangedFilesWorkbench, useChangedFileNavigation, type ChangedFilesProps } from './changed-files-workbench'
import { UpdateDetailSkeleton } from './update-detail-skeleton'
import {
  compactHistorySourceId,
  historyCommitTitle,
  historyEntryCountLabel,
  historyEntryKindLabel,
} from './history-row-labels'
import { VisibilityChanges, type HistoryVisibilityChange } from './history-visibility-changes'
import type { HistoryEntryDetailResponse } from '@/api/types.generated'
import type { ReactNode } from 'react'

export function HistoryEntryDetailPanel(props: ChangedFilesProps & {
  resource: CachedResource<HistoryEntryDetailResponse>
  onRetryDetail: () => void
  onSelectVisibility: (change: HistoryVisibilityChange) => void
  selectedVisibilityId: string | null
}) {
  const { resource, onCloseDiff, onRetryDetail, onSelectVisibility, selectedFilePath, selectedVisibilityId } = props
  // With nothing selected the file list is the content, so small screens start with it open.
  const navigation = useChangedFileNavigation(onCloseDiff, selectedFilePath === null)
  if (resource.status === 'loading') {
    return (
      <PendingSurface label="Loading update details" onRetry={onRetryDetail}>
        <UpdateDetailSkeleton />
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
  if (!resource.value) return null
  const detail = resource.value
  return (
    <div className="scope-content-enter min-w-0">
      <header className="border-b border-border px-5 py-5 sm:px-6">
        <h1 className="break-words text-lg font-semibold leading-6 tracking-[-0.01em]">
          {historyCommitTitle(detail)}
        </h1>
        <p className="mt-1.5 flex flex-wrap items-center gap-x-2 gap-y-1 text-xs text-muted-foreground">
          {metadataItems(detail).map((item, index) => (
            <span className="flex items-center gap-2" key={index}>
              {index > 0 ? <span aria-hidden="true">·</span> : null}
              {item}
            </span>
          ))}
        </p>
      </header>
      <VisibilityChanges
        changes={detail.visibility_changes}
        onSelect={onSelectVisibility}
        selectedId={selectedVisibilityId}
      />
      <ChangedFilesWorkbench
        {...props}
        expandFolders
        files={detail.files}
        navigation={navigation}
        navigationLabel="Update file navigator"
      />
    </div>
  )
}

function metadataItems(detail: HistoryEntryDetailResponse): ReactNode[] {
  const count = historyEntryCountLabel(detail)
  return [
    detail.kind === 'push' ? null : historyEntryKindLabel(detail.kind),
    detail.author,
    detail.occurred_at_unix === null ? null : <AbsoluteTimestamp value={detail.occurred_at_unix} />,
    <span className="select-all font-mono text-[11px]" title={detail.source_id}>
      {compactHistorySourceId(detail.source_id)}
    </span>,
    count || null,
  ].filter((item) => item !== null)
}
