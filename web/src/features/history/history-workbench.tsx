import type { CommitSummary } from '@/api/types'
import type { CommitFileResponse } from '@/api/types.generated'
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

/** One revision's commits: a header, the commit picker when there is a choice, then files and diff. */
export function HistoryWorkbench({
  commitContext,
  commitState,
  commits,
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
  diffIdentity: string | null
  emptyDescription: string
  emptyTitle: string
  fileDiffState: CommitFileDiffState
  header?: ReactNode
  onCloseDiff: () => void
  onRetryDiff?: () => void
  onSelectCommit: (commit: CommitSummary) => void
  onSelectFile: (file: CommitFileResponse) => void
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
              <h2 className="px-5 pt-3 pb-1 text-xs font-medium text-muted-foreground sm:px-6 lg:px-8">
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
