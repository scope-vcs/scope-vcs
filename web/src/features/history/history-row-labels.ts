import type { CommitSummary } from '@/api/types'
import type {
  HistoryEntryKind,
  HistoryEntrySummaryResponse,
} from '@/api/types.generated'

type HistoryRowCommit = Pick<
  CommitSummary,
  'change_count' | 'logical_commit_id' | 'message'
>

const REVIEWED_PUSH_ID = /^rv_push_([0-9a-f]{40})$/

export function historyRowLabels(commit: HistoryRowCommit) {
  const title = historyCommitTitle(commit)
  const reviewedPush = REVIEWED_PUSH_ID.exec(commit.logical_commit_id)
  const compactId = reviewedPush
    ? reviewedPush[1].slice(0, 12)
    : commit.logical_commit_id
  const fileCount = `${commit.change_count} ${commit.change_count === 1 ? 'file' : 'files'}`

  return {
    ariaLabel: `${title}, commit ${commit.logical_commit_id}, ${fileCount}`,
    compactId,
    title,
  }
}

export function historyCommitTitle(commit: Pick<CommitSummary, 'message'>) {
  return commit.message.split(/\r?\n/, 1)[0]?.trim() || '(no message)'
}

export function historyEntryLabels(entry: HistoryEntrySummaryResponse, viewName: string) {
  return {
    count: historyEntryCountLabel(entry, viewName),
    kind: entry.kind === 'push' ? null : historyEntryKindLabel(entry.kind),
    title: historyCommitTitle(entry),
  }
}

export function historyEntryKindLabel(kind: HistoryEntryKind) {
  switch (kind) {
    case 'push':
      return 'Push'
    case 'merged_request':
      return 'Merged'
    case 'visibility_change':
      return 'Visibility'
    case 'views_change':
      return 'Views'
  }
}

export function compactHistorySourceId(sourceId: string) {
  const reviewedPush = REVIEWED_PUSH_ID.exec(sourceId)
  return reviewedPush ? reviewedPush[1].slice(0, 12) : sourceId
}

export function historyEntryCountLabel(entry: Pick<HistoryEntrySummaryResponse, 'file_change_count' | 'kind' | 'visibility_summary'>, viewName: string) {
  const files = entry.kind === 'push' || entry.kind === 'merged_request' ? entry.file_change_count : 0
  const { entered_count: entered, left_count: left } = entry.visibility_summary
  const name = viewName.toLowerCase()
  return [
    files > 0 ? `${files} ${files === 1 ? 'file' : 'files'}` : null,
    entered > 0 ? `${entered} entered ${name} view` : null,
    left > 0 ? `${left} left ${name} view` : null,
  ].filter(Boolean).join(', ')
}
