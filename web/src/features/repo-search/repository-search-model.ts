import type { HistoryEntrySummaryResponse, HistoryFeed, RepoFileResponse, RequestQueueItemResponse } from '../../api/types.generated'
import { displayRouteFilePath } from '../../lib/route-file'
import { matchingRepositoryFiles } from '../repo-detail/repository-file-navigation'
import type { RepositoryVersion } from '../repo-detail/repository-version'

export type RepositorySearchFilter = 'all' | 'requests' | 'updates' | 'visibility' | 'files'

export type RepositorySearchResults = {
  history: HistoryEntrySummaryResponse[]
  requests: RequestQueueItemResponse[]
}

export type RepositorySearchRow =
  | { id: string; kind: 'main' }
  | { id: string; kind: 'request'; item: RequestQueueItemResponse }
  | { id: string; kind: 'history'; entry: HistoryEntrySummaryResponse }
  | { id: string; kind: 'file'; path: string }

export type RepositorySearchGroup = {
  id: string
  label: string
  rows: RepositorySearchRow[]
  total: number
}

const SUMMARY_LIMIT = 5
const FILTERED_LIMIT = 30

export function historyEntryOpensCode(entry: Pick<HistoryEntrySummaryResponse, 'kind'>) {
  return entry.kind === 'push' || entry.kind === 'merged_request'
}

export function repositorySearchFeed(filter: RepositorySearchFilter): HistoryFeed {
  if (filter === 'updates') return 'updates'
  return filter === 'visibility' ? 'visibility' : 'all'
}

export function repositoryFilesLabel(version: RepositoryVersion) {
  if (version.kind === 'request') return 'Files in this request'
  if (version.kind === 'update') return 'Files at this update'
  return 'Files on main'
}

export function repositorySearchGroups({
  filter,
  files,
  query,
  results,
  version,
}: {
  filter: RepositorySearchFilter
  files: readonly RepoFileResponse[]
  query: string
  results: RepositorySearchResults
  version: RepositoryVersion
}): RepositorySearchGroup[] {
  const needle = query.trim().toLowerCase()
  const limit = filter === 'all' ? SUMMARY_LIMIT : FILTERED_LIMIT
  const groups: RepositorySearchGroup[] = []
  const requests = results.requests.map((item): RepositorySearchRow => ({
    id: `request:${item.request.id}`,
    item,
    kind: 'request',
  }))
  const showsMain = filter === 'all' && (!needle || 'main'.includes(needle))

  if (filter === 'all' || filter === 'requests') {
    groups.push({
      id: 'requests',
      label: needle || filter === 'requests' ? 'Requests' : 'Versions',
      rows: [
        ...(showsMain ? [{ id: 'main', kind: 'main' } as const] : []),
        ...(filter === 'all' ? requests.slice(0, limit) : requests),
      ],
      total: requests.length + (showsMain ? 1 : 0),
    })
  }
  if (filter === 'all' || filter === 'updates' || filter === 'visibility') {
    const history = results.history.map((entry): RepositorySearchRow => ({
      entry,
      id: `history:${entry.source_id}`,
      kind: 'history',
    }))
    groups.push({
      id: 'history',
      label: 'History',
      rows: filter === 'all' ? history.slice(0, limit) : history,
      total: history.length,
    })
  }
  if (filter === 'files' || (filter === 'all' && needle)) {
    const { shown, total } = matchingRepositoryFiles(files, needle)
    groups.push({
      id: 'files',
      label: repositoryFilesLabel(version),
      rows: shown.slice(0, limit).map((file) => ({
        id: `file:${file.path}`,
        kind: 'file',
        path: displayRouteFilePath(file.path),
      })),
      total,
    })
  }
  return groups.filter((group) => group.rows.length > 0)
}
