import type { CommitFile, CommitSummary } from '@/api/types'
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
import { useState, type ReactNode } from 'react'

export function HistoryWorkbench({
  commitContext,
  commitState,
  commits,
  diffIdentity,
  emptyDescription,
  emptyTitle,
  fileDiffState,
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
  onCloseDiff: () => void
  onRetryDiff?: () => void
  onSelectCommit: (commit: CommitSummary) => void
  onSelectFile: (file: CommitFile) => void
  selectedCommitId: string | null
  selectedFilePath: string | null
}) {
  const [commitsOpen, setCommitsOpen] = useState(false)
  return (
    <section className="border-t border-border">
      {commits.length === 0 ? (
        <EmptyState
          description={emptyDescription}
          icon={<History />}
          title={emptyTitle}
        />
      ) : (
        <div>
          <details className="border-b border-border" open={commitsOpen} onToggle={(event) => setCommitsOpen(event.currentTarget.open)}>
            <summary className="cursor-pointer px-5 py-3 text-sm font-medium">commits · {commits.length}</summary>
            <div className="max-h-72 overflow-y-auto">
          <CommitList
            commits={commits}
            onSelectCommit={(commit) => { onSelectCommit(commit); setCommitsOpen(false) }}
            selectedCommitId={selectedCommitId}
          />
            </div>
          </details>
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
