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

export function historyEntryLabels(entry: HistoryEntrySummaryResponse) {
  const title = historyCommitTitle(entry)
  const kind = historyEntryKindLabel(entry.kind)
  const counts = historyEntryCountLabel(entry)
  return {
    ariaLabel: `${kind}: ${title}, update ${entry.source_id}, ${counts}`,
    compactId: compactHistorySourceId(entry.source_id),
    count: counts,
    kind,
    title,
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
  }
}

function compactHistorySourceId(sourceId: string) {
  const reviewedPush = REVIEWED_PUSH_ID.exec(sourceId)
  return reviewedPush ? reviewedPush[1].slice(0, 12) : sourceId
}

export function historyEntryCountLabel(entry: Pick<HistoryEntrySummaryResponse, 'file_change_count' | 'kind' | 'visibility_summary'>) {
  const files = entry.kind === 'visibility_change' ? 0 : entry.file_change_count
  const { made_public_count: madePublic, made_private_count: madePrivate } = entry.visibility_summary
  return [
    files > 0 ? `${files} file ${files === 1 ? 'change' : 'changes'}` : null,
    madePublic > 0 ? `${madePublic} made public` : null,
    madePrivate > 0 ? `${madePrivate} made private` : null,
  ].filter(Boolean).join(', ')
}
