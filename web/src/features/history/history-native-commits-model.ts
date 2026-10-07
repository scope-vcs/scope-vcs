import type { RepoViews } from '../../api/repo-views'
import type { HistoryEntryDetailResponse } from '../../api/types.generated'
import { shortOid } from '../../lib/short-oid'
import { historyCommitTitle } from './history-row-labels'

export type HistoryNativeCommitRow = {
  oid: string
  shortOid: string
  title: string
  author: string
  fileCount: string
}

export function historyNativeCommitsHeading(
  detail: Pick<HistoryEntryDetailResponse, 'native_commits'>,
  views: Pick<RepoViews, 'name'>,
) {
  return detail.native_commits
    ? `Original request commits from the ${views.name(detail.native_commits.view)} view`
    : 'Request commits'
}

export function historyNativeCommitRows(
  detail: Pick<HistoryEntryDetailResponse, 'native_commits'>,
): HistoryNativeCommitRow[] {
  return (detail.native_commits?.commits ?? []).map((commit) => ({
    oid: commit.oid,
    shortOid: shortOid(commit.oid),
    title: historyCommitTitle(commit),
    author: commit.author,
    fileCount: `${commit.files.length} ${commit.files.length === 1 ? 'file' : 'files'}`,
  }))
}
