import { PanelState } from '@/components/empty-state'
import { historyCommitTitle } from '@/features/history/history-row-labels'
import type { CommitDetailState } from '@/features/history/history-state'
import { GitCommit, TriangleAlert } from 'lucide-react'
import type { ReactNode } from 'react'
import { ChangedFilesWorkbench, useChangedFileNavigation, type ChangedFilesProps } from './changed-files-workbench'

type CommitDetailPanelProps = ChangedFilesProps & {
  commitContext?: ReactNode
  commitState: CommitDetailState
}

export function CommitDetailPanel(props: CommitDetailPanelProps) {
  const { commitContext, commitState, onCloseDiff, selectedFilePath } = props
  // With nothing selected the file list is the content, so small screens start with it open.
  const navigation = useChangedFileNavigation(onCloseDiff, selectedFilePath === null)

  if (commitState.status === 'failed') {
    return (
      <PanelState tone="error">
        <TriangleAlert className="size-5" />
        <span>{commitState.error}</span>
      </PanelState>
    )
  }

  if (!commitState.commit) {
    return (
      <PanelState>
        <GitCommit className="size-5" />
        <span>Select a commit</span>
      </PanelState>
    )
  }

  const commit = commitState.commit
  const filesTruncated = commit.files_truncated
  return (
    <div className="scope-content-enter min-w-0">
      <div className="border-b border-border px-5 py-4 sm:px-6 lg:px-8">
        <h3 className="break-words text-sm font-semibold leading-5">
          {historyCommitTitle(commit)}
        </h3>
        <div className="mt-1.5 flex flex-wrap items-center gap-x-2 gap-y-1 font-mono text-xs text-muted-foreground">
          <span>{commit.logical_commit_id}</span>
          {commit.author && (
            <>
              <span aria-hidden>·</span>
              <span>{commit.author}</span>
            </>
          )}
        </div>
        {commitContext}
      </div>

      <ChangedFilesWorkbench
        {...props}
        expandFolders
        files={commit.files}
        navigation={navigation}
        navigationLabel="Commit file navigator"
        emptyFilesMessage={filesTruncated
          ? `${commit.change_count} changed files are outside the bounded file list.`
          : 'No file changes in this commit.'}
        emptyPreviewMessage={commit.files.length === 0 && filesTruncated
          ? 'Changed files are outside the bounded file list'
          : undefined}
      />
    </div>
  )
}
