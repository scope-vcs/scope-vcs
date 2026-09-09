import type { CommitFile } from '@/api/types'
import { EmptyState, PanelState } from '@/components/empty-state'
import { FileSystemTree } from '@/components/file-system-tree'
import { FileWorkbench } from '@/components/file-workbench'
import { Badge } from '@/components/ui/badge'
import { useRef, useState } from 'react'
import { ReviewFileDiffDrawer } from '../review/review-file-diff-drawer'
import type { CommitFileDiffState } from './history-state'

export type ChangedFilesProps = {
  diffIdentity: string | null
  diffScrollTop: number
  fileDiffState: CommitFileDiffState
  onCloseDiff: () => void
  onDiffScroll: (scrollTop: number) => void
  onRetryDiff?: () => void
  onSelectFile: (file: CommitFile) => void
  selectedFilePath: string | null
}

// Keep navigation state in the detail panel so loading and error surfaces do
// not discard it while the changed-file content is temporarily hidden.
export function useChangedFileNavigation(onCloseDiff: () => void) {
  const [open, setOpen] = useState(false)
  const ref = useRef<HTMLDivElement>(null)
  function closeDiff() {
    onCloseDiff()
    setOpen(true)
    requestAnimationFrame(() => ref.current?.focus())
  }
  return { open, setOpen, ref, closeDiff }
}

export function ChangedFilesWorkbench({
  diffIdentity, diffScrollTop, fileDiffState, onDiffScroll, onRetryDiff,
  onSelectFile, selectedFilePath, files, navigation, navigationLabel,
  selectedVisibilityId, emptyFilesMessage, emptyPreviewMessage,
}: Omit<ChangedFilesProps, 'onCloseDiff'> & {
  files: CommitFile[]
  navigation: ReturnType<typeof useChangedFileNavigation>
  navigationLabel: string
  selectedVisibilityId?: string | null
  // Commit details retain their empty file navigator and bounded preview pane.
  // Entries with only visibility changes instead render just the selected diff.
  emptyFilesMessage?: string
  emptyPreviewMessage?: string
}) {
  const placeholder = <PanelState><span>{emptyPreviewMessage ?? 'Select a changed file'}</span></PanelState>
  const preview = selectedFilePath || emptyFilesMessage !== undefined ? (
    <div className="h-[70vh] min-h-[340px] max-h-[720px] min-w-0 overflow-hidden">
      {selectedFilePath ? (
        <ReviewFileDiffDrawer
          cacheKey={diffIdentity}
          diff={fileDiffState.diff}
          error={fileDiffState.error}
          loading={fileDiffState.status === 'loading'}
          onClose={navigation.closeDiff}
          onRetry={fileDiffState.status === 'failed' ? onRetryDiff : undefined}
          onScrollTopChange={onDiffScroll}
          scrollTop={diffScrollTop}
          selectedPath={selectedFilePath}
        />
      ) : placeholder}
    </div>
  ) : null
  if (!files.length && emptyFilesMessage === undefined) return preview
  return (
    <FileWorkbench
      navigationOpen={navigation.open}
      onNavigationOpenChange={navigation.setOpen}
      selectedPath={selectedFilePath}
    >
      <div
        aria-label={navigationLabel}
        className="min-w-0 outline-none focus-visible:ring-2 focus-visible:ring-inset focus-visible:ring-ring"
        ref={navigation.ref}
        tabIndex={-1}
      >
        {files.length ? (
          <FileSystemTree
            compactVisibility
            files={files}
            getFileMeta={(file) => <Badge variant="neutral">{file.kind}</Badge>}
            metaColumnLabel="change"
            onSelectFile={(file) => {
              onSelectFile(file)
              navigation.setOpen(false)
            }}
            selectedFilePath={selectedVisibilityId ? null : selectedFilePath}
          />
        ) : <EmptyState inline className="px-5 py-8 sm:px-6" title={emptyFilesMessage ?? ''} />}
      </div>
      {preview ?? placeholder}
    </FileWorkbench>
  )
}
