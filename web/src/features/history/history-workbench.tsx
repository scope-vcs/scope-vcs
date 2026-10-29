import type { CommitSummary } from '@/api/types'
import { EmptyState } from '@/components/empty-state'
import { CommitDetailPanel } from '@/features/history/history-commit-detail'
import { CommitList } from '@/features/history/history-commit-list'
import {
  readHistoryDiffScroll,
  writeHistoryDiffScroll,
} from '@/features/history/history-resource-cache'
import type {
  CommitDetailState,
  CommitFileDiffState,
} from '@/features/history/history-state'
import { History } from 'lucide-react'
import type { ReactNode } from 'react'
import type { ReviewDiffAnnotations } from '@/features/review/review-file-diff-drawer'

export function HistoryWorkbench({
  commitContext,
  commitState,
  commits,
  diffAnnotations,
  diffIdentity,
  emptyDescription,
  emptyTitle,
  fileDiffState,
  header,
  onCloseDiff,
  onRetryDiff,
  onSelectCommit,
  onSelectFile,
  selectedCommitId,
  selectedFilePath,
}: {
  commitContext?: ReactNode
  commitState: CommitDetailState
  commits: CommitSummary[]
  diffAnnotations?: ReviewDiffAnnotations
  diffIdentity: string | null
  emptyDescription: string
  emptyTitle: string
  fileDiffState: CommitFileDiffState
  header?: ReactNode
  onCloseDiff: () => void
  onRetryDiff?: () => void
  onSelectCommit: (commit: CommitSummary) => void
  onSelectFile: (path: string) => void
  selectedCommitId: string | null
  selectedFilePath: string | null
}) {
  return (
    <section>
      {header}
      {commits.length === 0 ? (
        <EmptyState
          description={emptyDescription}
          icon={<History />}
          title={emptyTitle}
        />
      ) : (
        <div>
          {commits.length > 1 ? (
            <div className="border-b border-border">
              <h2 className="px-(--app-inset) pt-3 pb-1 text-xs font-medium text-muted-foreground">
                {commits.length} commits in this revision
              </h2>
              <div className="max-h-72 overflow-y-auto">
                <CommitList
                  commits={commits}
                  onSelectCommit={onSelectCommit}
                  selectedCommitId={selectedCommitId}
                />
              </div>
            </div>
          ) : null}
          <CommitDetailPanel
            commitContext={commitContext}
            commitState={commitState}
            diffAnnotations={diffAnnotations}
            diffIdentity={diffIdentity}
            diffScrollTop={readHistoryDiffScroll(diffIdentity)}
            fileDiffState={fileDiffState}
            onCloseDiff={onCloseDiff}
            onDiffScroll={(scrollTop) => writeHistoryDiffScroll(diffIdentity, scrollTop)}
            onRetryDiff={onRetryDiff}
            onSelectFile={onSelectFile}
            selectedFilePath={selectedFilePath}
          />
        </div>
      )}
    </section>
  )
}
