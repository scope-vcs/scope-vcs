import type { CommitFile, HistoryEntryDetail } from '@/api/types'
import { PanelState } from '@/components/empty-state'
import { FileWorkbench } from '@/components/file-workbench'
import { FileSystemTree } from '@/components/file-system-tree'
import { Badge } from '@/components/ui/badge'
import { Button } from '@/components/ui/button'
import { PendingSurface } from '@/components/pending-surface'
import type { CachedResource } from '@/lib/use-cached-resource'
import { useRef, useState } from 'react'
import { ReviewFileDiffDrawer } from '../review/review-file-diff-drawer'
import { CommitDetailSkeleton } from './history-commit-detail-skeleton'
import { historyCommitTitle, historyEntryCountLabel, historyEntryKindLabel } from './history-row-labels'
import type { CommitFileDiffState } from './history-state'
import { VisibilityChanges, type HistoryVisibilityChange } from './history-visibility-changes'

export function HistoryEntryDetailPanel({
  resource,
  diffIdentity,
  diffScrollTop,
  fileDiffState,
  onCloseDiff,
  onDiffScroll,
  onRetryDetail,
  onRetryDiff,
  onSelectFile,
  onSelectVisibility,
  selectedFilePath,
  selectedVisibilityId,
}: {
  resource: CachedResource<HistoryEntryDetail>
  diffIdentity: string | null
  diffScrollTop: number
  fileDiffState: CommitFileDiffState
  onCloseDiff: () => void
  onDiffScroll: (scrollTop: number) => void
  onRetryDetail: () => void
  onRetryDiff?: () => void
  onSelectFile: (file: CommitFile) => void
  onSelectVisibility: (change: HistoryVisibilityChange) => void
  selectedFilePath: string | null
  selectedVisibilityId: string | null
}) {
  const fileNavigatorRef = useRef<HTMLDivElement>(null)
  const [navigationOpen, setNavigationOpen] = useState(false)
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
  const preview = selectedFilePath ? (
    <div className="h-[70vh] min-h-[340px] max-h-[720px] min-w-0 overflow-hidden">
      <ReviewFileDiffDrawer
        cacheKey={diffIdentity}
        diff={fileDiffState.diff}
        error={fileDiffState.error}
        loading={fileDiffState.status === 'loading'}
        onClose={() => {
          onCloseDiff()
          setNavigationOpen(true)
          requestAnimationFrame(() => fileNavigatorRef.current?.focus())
        }}
        onRetry={fileDiffState.status === 'failed' ? onRetryDiff : undefined}
        onScrollTopChange={onDiffScroll}
        scrollTop={diffScrollTop}
        selectedPath={selectedFilePath}
      />
    </div>
  ) : null
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
      {detail.files.length > 0 ? (
        <FileWorkbench
          navigationOpen={navigationOpen}
          onNavigationOpenChange={setNavigationOpen}
          selectedPath={selectedFilePath}
        >
          <div
            aria-label="Update file navigator"
            ref={fileNavigatorRef}
            tabIndex={-1}
            className="outline-none focus-visible:ring-2 focus-visible:ring-inset focus-visible:ring-ring"
          >
            <FileSystemTree
              compactVisibility
              files={detail.files}
              getFileMeta={(file) => <Badge variant="neutral">{file.kind}</Badge>}
              metaColumnLabel="change"
              onSelectFile={(file) => {
                onSelectFile(file)
                setNavigationOpen(false)
              }}
              selectedFilePath={selectedVisibilityId ? null : selectedFilePath}
            />
          </div>
          {preview ?? <PanelState><span>Select a changed file</span></PanelState>}
        </FileWorkbench>
      ) : preview}
    </div>
  )
}
