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
  detail: Pick<HistoryEntryDetailResponse, 'view'>,
  views: Pick<RepoViews, 'full' | 'name'>,
) {
  return detail.view === views.full
    ? 'Request commits'
    : `Request commits preserved in the ${views.name(detail.view)} view`
}

export function historyNativeCommitRows(
  detail: Pick<HistoryEntryDetailResponse, 'native_commits'>,
): HistoryNativeCommitRow[] {
  return detail.native_commits.map((commit) => ({
    oid: commit.oid,
    shortOid: shortOid(commit.oid),
    title: historyCommitTitle(commit),
    author: commit.author,
    fileCount: `${commit.files.length} ${commit.files.length === 1 ? 'file' : 'files'}`,
  }))
}
