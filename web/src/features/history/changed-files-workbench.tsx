import { EmptyState, PanelState } from '@/components/empty-state'
import { FileWorkbench } from '@/components/file-workbench'
import { useRef, useState, type ReactNode } from 'react'
import { ReviewFileDiffDrawer, type ReviewDiffAnnotations } from '../review/review-file-diff-drawer'
import type { CommitFileDiffState } from './history-state'

export type ChangedFilesProps = {
  diffAnnotations?: ReviewDiffAnnotations
  diffIdentity: string | null
  diffScrollTop: number
  fileDiffState: CommitFileDiffState
  onCloseDiff: () => void
  onDiffScroll: (scrollTop: number) => void
  onRetryDiff?: () => void
  onSelectFile: (path: string) => void
  selectedFilePath: string | null
}

export function useChangedFileNavigation(onCloseDiff: () => void, initiallyOpen = false) {
  const [open, setOpen] = useState(initiallyOpen)
  const ref = useRef<HTMLDivElement>(null)
  function closeDiff() {
    onCloseDiff()
    setOpen(true)
    requestAnimationFrame(() => ref.current?.focus())
  }
  return { open, setOpen, ref, closeDiff }
}

const DIFF_PANE_CLASS = 'h-[70vh] min-h-[340px] max-h-[720px] min-w-0 overflow-hidden'

export function ChangedFilesWorkbench({
  diffAnnotations, diffIdentity, diffScrollTop, fileDiffState, onDiffScroll, onRetryDiff,
  selectedFilePath, navigation, navigationLabel, navigator, toolbar, diffActions, diffContext,
  emptyFilesMessage, emptyPreviewMessage,
}: Omit<ChangedFilesProps, 'onCloseDiff' | 'onSelectFile'> & {
  navigation: ReturnType<typeof useChangedFileNavigation>
  navigationLabel: string
  navigator: ReactNode
  toolbar?: ReactNode
  diffActions?: ReactNode
  diffContext?: ReactNode
  emptyFilesMessage?: string
  emptyPreviewMessage?: string
}) {
  const placeholder = <PanelState><span>{emptyPreviewMessage ?? 'Select a changed file'}</span></PanelState>
  const diff = selectedFilePath ? (
    <div className={DIFF_PANE_CLASS}>
      <ReviewFileDiffDrawer
        actions={diffActions}
        annotations={diffAnnotations}
        cacheKey={diffIdentity}
        context={diffContext}
        diff={fileDiffState.diff}
        error={fileDiffState.error}
        fullRender={fileDiffState.status === 'loaded' ? fileDiffState.fullRender : null}
        loading={fileDiffState.status === 'loading'}
        onClose={navigation.closeDiff}
        onRetry={onRetryDiff}
        onScrollTopChange={onDiffScroll}
        scrollTop={diffScrollTop}
        selectedPath={selectedFilePath}
      />
    </div>
  ) : null
  if (!navigator && emptyFilesMessage === undefined) return diff
  const desktopPlaceholder = emptyFilesMessage === undefined
    ? placeholder
    : <div className={DIFF_PANE_CLASS}>{placeholder}</div>
  return (
    <>
      {toolbar}
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
          {navigator || <EmptyState inline className="py-8" title={emptyFilesMessage ?? ''} />}
        </div>
        {diff ?? <div className="hidden lg:contents">{desktopPlaceholder}</div>}
      </FileWorkbench>
    </>
  )
}
