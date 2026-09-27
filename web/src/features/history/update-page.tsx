import type { RepoParams } from '@/api/types'
import type {
  CommitFileResponse,
  HistoryEntryDetailResponse,
  ProjectionPreviewAudience,
} from '@/api/types.generated'
import { WorkbenchPane } from '@/components/page-header'
import { useRepoLayout } from '@/features/repo-detail/repo-layout-context'
import { repoResourceScope } from '@/features/repo-detail/repo-resource-scope'
import { useCachedResource } from '@/lib/use-cached-resource'
import { loadHistoryEntry, loadHistoryEntryFileDiff } from '@/routes/-repo-history-actions'
import { useAuth } from '@clerk/tanstack-react-start'
import { useNavigate } from '@tanstack/react-router'
import { useCallback } from 'react'
import { defaultHistoryAudience } from './history-feed'
import { HistoryEntryDetailPanel } from './history-entry-detail'
import {
  historyDiffResource,
  historyEntryCacheKey,
  historyEntryDiffCacheKey,
  historyEntryResource,
  readHistoryDiffScroll,
  writeHistoryDiffScroll,
} from './history-resource-cache'
import { historyFileSelection } from './history-selection'
import { resourceToDiffState, type CommitFileDiffState } from './history-state'
import type { HistoryVisibilityChange } from './history-visibility-changes'
import { UpdateNavigation } from './update-navigation'
import { updateAudienceSearch, type UpdateSearch } from './update-search'

type UpdatePageProps = {
  initialEntry: HistoryEntryDetailResponse
  initialEntryScope: string | null
  params: RepoParams & { entryId: string }
  search: UpdateSearch
}

export function UpdatePage(props: UpdatePageProps) {
  const {
    audienceSearch,
    closeDiff,
    diffIdentity,
    entryResource,
    fileDiffState,
    retryDiff,
    selectFile,
    selectVisibility,
    selectedFilePath,
    selectedVisibilityId,
  } = useUpdatePageModel(props)
  const { params } = props
  const repoParams = { owner: params.owner, repo: params.repo }
  const detail = entryResource.value

  return (
    <WorkbenchPane>
      <UpdateNavigation
        newer={detail?.newer_source_id ?? null}
        older={detail?.older_source_id ?? null}
        params={repoParams}
        search={audienceSearch}
      />
      <HistoryEntryDetailPanel
        diffIdentity={diffIdentity}
        diffScrollTop={readHistoryDiffScroll(diffIdentity)}
        fileDiffState={fileDiffState}
        key={params.entryId}
        onCloseDiff={closeDiff}
        onDiffScroll={(scrollTop) => writeHistoryDiffScroll(diffIdentity, scrollTop)}
        onRetryDetail={entryResource.retry}
        onRetryDiff={retryDiff}
        onSelectFile={selectFile}
        onSelectVisibility={selectVisibility}
        resource={entryResource}
        selectedFilePath={selectedFilePath}
        selectedVisibilityId={selectedVisibilityId}
      />
    </WorkbenchPane>
  )
}

function useUpdatePageModel({ initialEntry, initialEntryScope, params, search }: UpdatePageProps) {
  const navigate = useNavigate()
  const { userId, isLoaded } = useAuth()
  const { repo } = useRepoLayout()
  const scope = isLoaded ? repoResourceScope(repo, userId ?? null) : null
  const audience: ProjectionPreviewAudience = search.audience ?? initialEntry.audience
  const audienceSearch = updateAudienceSearch(
    audience,
    defaultHistoryAudience(repo.access.can_read_private_files),
  )
  const { owner, repo: repoName, entryId } = params
  const version = String(repo.change_version)
  const entryIdentity = scope ? historyEntryCacheKey({ scope, audience, entry: entryId }) : null
  const loadEntry = useCallback(
    (signal: AbortSignal) => loadHistoryEntry({
      data: { audience, entry: entryId, owner, repo: repoName },
      signal,
    }).then((result) => result.entry),
    [audience, entryId, owner, repoName],
  )
  const entryResource = useCachedResource({
    fallbackError: 'This update is unavailable.',
    identity: entryIdentity,
    // The loader already read this entry for this viewer; seed rather than refetch.
    initialValue: scope === initialEntryScope && initialEntry.source_id === entryId ? initialEntry : null,
    load: loadEntry,
    resource: historyEntryResource,
    version,
  })
  const selectedEntry = entryResource.value
  const { path: selectedFilePath, file: selectedFile, visibilityId: selectedVisibilityId } =
    historyFileSelection(search, selectedEntry)
  const diffIdentity = scope && selectedFile
    ? historyEntryDiffCacheKey({
        scope,
        audience,
        entry: entryId,
        visibilityChange: selectedVisibilityId,
        newOid: selectedFile.new_oid,
        oldOid: selectedFile.old_oid,
        path: selectedFile.path,
      })
    : null
  const loadDiff = useCallback(
    (signal: AbortSignal) => loadHistoryEntryFileDiff({
      data: {
        audience,
        entry: entryId,
        owner,
        path: selectedFilePath ?? '',
        repo: repoName,
        visibility_change: selectedVisibilityId,
      },
      signal,
    }),
    [audience, entryId, owner, repoName, selectedFilePath, selectedVisibilityId],
  )
  const diffResource = useCachedResource({
    fallbackError: 'This file diff is unavailable.',
    identity: diffIdentity,
    load: loadDiff,
    resource: historyDiffResource,
  })
  const missingFile = selectedFilePath !== null && selectedEntry !== null && !selectedFile
  const fileDiffState: CommitFileDiffState = missingFile
    ? { diff: null, error: selectedVisibilityId ? 'This visibility preview is unavailable.' : 'This file is not part of the selected update.', status: 'failed' }
    : resourceToDiffState(diffResource)

  const selectPath = useCallback(
    (path: string | null, visibilityId: string | null = null) => navigate({
      params,
      replace: true,
      resetScroll: false,
      search: (current) => ({
        ...current,
        path: path ?? undefined,
        visibility_change: visibilityId ?? undefined,
      }),
      to: '/$owner/$repo/updates/$entryId',
    }),
    [navigate, params],
  )

  const closeDiff = useCallback(() => void selectPath(null), [selectPath])
  const selectFile = useCallback((file: CommitFileResponse) => void selectPath(file.path), [selectPath])
  const selectVisibility = useCallback(
    (change: HistoryVisibilityChange) => void selectPath(change.path, change.id),
    [selectPath],
  )

  return {
    audienceSearch,
    closeDiff,
    diffIdentity,
    entryResource,
    fileDiffState,
    retryDiff: missingFile ? undefined : diffResource.retry,
    selectFile,
    selectVisibility,
    selectedFilePath,
    selectedVisibilityId,
  }
}
