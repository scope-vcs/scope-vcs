import type {
  CommitFile,
  HistoryEntrySummary,
  HistoryEntryDetail,
  HistoryPage as HistoryPageResponse,
  ProjectionPreviewAudience,
  RepoParams,
} from '@/api/types'
import { WorkbenchBar, WorkbenchPane } from '@/components/page-header'
import { AudienceToggle } from '@/features/history/history-audience-toggle'
import { HistoryEntryDetailPanel } from './history-entry-detail'
import { ToggleGroup, ToggleGroupItem } from '@/components/ui/toggle-group'
import { Button } from '@/components/ui/button'
import type { HistoryVisibilityChange } from './history-visibility-changes'
import { HistoryEntryList } from '@/features/history/history-entry-list'
import {
  appendHistoryPage,
  historySummary,
} from '@/features/history/history-pagination'
import {
  historyEntryCacheKey,
  historyEntryDiffCacheKey,
  historyDiffResource,
  historyEntryResource,
  readHistoryDiffScroll,
  writeHistoryDiffScroll,
} from '@/features/history/history-resource-cache'
import {
  resourceToDiffState,
  type CommitFileDiffState,
} from '@/features/history/history-state'
import { useRepoLayout } from '@/features/repo-detail/repo-layout-context'
import { useCachedResource } from '@/lib/use-cached-resource'
import {
  loadHistoryEntry,
  loadHistoryEntryFileDiff,
  loadHistoryPage,
} from '@/routes/-repo-history-actions'
import { useLocation, useNavigate } from '@tanstack/react-router'
import { useCallback, useEffect, useState } from 'react'
import { useAuth } from '@clerk/tanstack-react-start'
import { repoResourceScope } from '../repo-detail/repo-resource-scope'
import { historyPageCacheKey, restoreHistoryPages, retainHistoryPages } from './history-page-cache'
import { historyFileSelection } from './history-selection'

export type HistorySearch = {
  audience?: ProjectionPreviewAudience
  feed?: 'updates' | 'all'
  visibility_change?: string
  entry?: string
  path?: string
}

type HistoryPageProps = {
  initialPage: HistoryPageResponse
  initialEntry: HistoryEntryDetail | null
  params: RepoParams
  search: HistorySearch
}

export function HistoryPage(props: HistoryPageProps) {
  const { userId, isLoaded } = useAuth()
  const { repo } = useRepoLayout()
  const cacheKey = isLoaded
    ? historyPageCacheKey(repoResourceScope(repo, userId ?? null), props.initialPage)
    : null
  return <HistoryPageContent key={cacheKey ?? 'pending'} {...props} cacheKey={cacheKey} />
}

function HistoryPageContent(props: HistoryPageProps & { cacheKey: string | null }) {
  const {
    audience,
    availableAudiences,
    closeDiff,
    entryResource,
    feed,
    selectFeed,
    selectVisibility,
    selectedVisibilityId,
    entries,
    fileDiffState,
    loadOlder,
    loadOlderError,
    loadingOlder,
    retryDetail,
    retryDiff,
    selectAudience,
    selectEntry,
    selectFile,
    selectedEntryId,
    selectedFilePath,
    showLoadOlder,
    diffIdentity,
    saveDiffScroll,
  } = useHistoryPageModel(props)

  return (
    <WorkbenchPane>
      <WorkbenchBar
        actions={availableAudiences.length > 1 ? (
          <AudienceToggle
            audience={audience}
            availableAudiences={availableAudiences}
            onSelect={selectAudience}
          />
        ) : undefined}
        summary={historySummary(entries, showLoadOlder)}
        title="history"
      />
      <section className="border-t border-border">
        <div className="border-b border-border px-5 py-3 sm:px-6">
          <ToggleGroup type="single" value={feed} onValueChange={(value) => {
            if (value === 'updates' || value === 'all') selectFeed(value)
          }} aria-label="History activity">
            <ToggleGroupItem value="updates">Pushes &amp; merges</ToggleGroupItem>
            <ToggleGroupItem value="all">All activity</ToggleGroupItem>
          </ToggleGroup>
        </div>
        {entries.length === 0 && !selectedEntryId ? (
          <div className="px-5 py-12 text-center sm:px-6">
            <h2 className="text-sm font-semibold">{feed === 'updates' ? 'No pushes or merges yet' : 'No activity yet'}</h2>
            <p className="mt-1 text-sm text-muted-foreground">
              {feed === 'updates' ? 'Visibility changes appear in All activity.' : 'History appears here after Scope applies an update.'}
            </p>
            {feed === 'updates' ? <Button className="mt-3" onClick={() => selectFeed('all')} variant="secondary" size="sm">All activity</Button> : null}
          </div>
        ) : (
          <div>
            <div aria-label="History updates" className="max-h-80 overflow-y-auto border-b border-border">
              <HistoryEntryList
                entries={entries}
                loadOlderError={loadOlderError}
                loadingOlder={loadingOlder}
                onLoadOlder={loadOlder}
                onSelectEntry={selectEntry}
                selectedEntryId={selectedEntryId}
                showLoadOlder={showLoadOlder}
              />
            </div>
            <HistoryEntryDetailPanel
              key={selectedEntryId}
              resource={entryResource}
              diffIdentity={diffIdentity}
              diffScrollTop={readHistoryDiffScroll(diffIdentity)}
              fileDiffState={fileDiffState}
              onCloseDiff={closeDiff}
              onDiffScroll={saveDiffScroll}
              onRetryDetail={retryDetail}
              onRetryDiff={retryDiff}
              onSelectFile={selectFile}
              onSelectVisibility={selectVisibility}
              selectedFilePath={selectedFilePath}
              selectedVisibilityId={selectedVisibilityId}
            />
          </div>
        )}
      </section>
    </WorkbenchPane>
  )
}

function useHistoryPageModel({ initialPage, initialEntry, params, search, cacheKey }: HistoryPageProps & { cacheKey: string | null }) {
  const navigate = useNavigate()
  const locationKey = useLocation({ select: (location) => location.state.__TSR_key })
  const [diffSelection, setDiffSelection] = useState({ locationKey, dismissed: false })
  if (diffSelection.locationKey !== locationKey) {
    setDiffSelection({ locationKey, dismissed: false })
  }
  const { repo } = useRepoLayout()
  const [loaded, setLoaded] = useState(() => restoreHistoryPages(cacheKey, initialPage))
  useEffect(() => retainHistoryPages(cacheKey, loaded), [cacheKey, loaded])
  const [loadingOlder, setLoadingOlder] = useState(false)
  const [loadOlderError, setLoadOlderError] = useState<string | null>(null)
  const feed = initialPage.feed
  const audience = initialPage.audience
  const availableAudiences: ProjectionPreviewAudience[] = repo.access.can_read_private_files
    ? ['private', 'public']
    : ['public']
  const selectedEntryId = search.entry ?? loaded.entries[0]?.source_id ?? null
  const entryIdentity = selectedEntryId
    ? historyEntryCacheKey({
        audience,
        entry: selectedEntryId,
        generation: initialPage.generation,
        repoId: initialPage.repo_id,
        viewKey: initialPage.view_key,
      })
    : null
  const loadSelectedEntry = useCallback(
    (signal: AbortSignal) => initialEntry?.source_id === selectedEntryId
      ? Promise.resolve(initialEntry)
      : loadHistoryEntry({
      data: {
        audience,
        entry: selectedEntryId ?? '',
        owner: params.owner,
        repo: params.repo,
      },
      signal,
    }),
    [audience, initialEntry, params.owner, params.repo, selectedEntryId],
  )
  const entryResource = useCachedResource({
    fallbackError: 'This history update is unavailable.',
    identity: entryIdentity,
    load: loadSelectedEntry,
    resource: historyEntryResource,
  })
  const selectedEntry = entryResource.value
  const { path: selectedFilePath, file: selectedFile, visibilityId: selectedVisibilityId } = historyFileSelection(
    search,
    selectedEntry,
    diffSelection.locationKey === locationKey && diffSelection.dismissed,
  )
  const diffIdentity = selectedEntryId && selectedFile
    ? historyEntryDiffCacheKey({
        audience,
        entry: selectedEntryId,
        generation: initialPage.generation,
        visibilityChange: selectedVisibilityId,
        newOid: selectedFile.new_oid,
        oldOid: selectedFile.old_oid,
        path: selectedFile.path,
        repoId: initialPage.repo_id,
        viewKey: initialPage.view_key,
      })
    : null
  const loadSelectedDiff = useCallback(
    (signal: AbortSignal) => loadHistoryEntryFileDiff({
      data: {
        audience,
        entry: selectedEntryId ?? '',
        owner: params.owner,
        path: selectedFilePath ?? '',
        visibility_change: selectedVisibilityId,
        repo: params.repo,
      },
      signal,
    }),
    [audience, params.owner, params.repo, selectedEntryId, selectedFilePath, selectedVisibilityId],
  )
  const diffResource = useCachedResource({
    fallbackError: 'This file diff is unavailable.',
    identity: diffIdentity,
    load: loadSelectedDiff,
    resource: historyDiffResource,
  })
  const fileDiffState: CommitFileDiffState =
    selectedFilePath && selectedEntry && !selectedFile
      ? { diff: null, error: selectedVisibilityId ? 'This visibility preview is unavailable.' : 'This file is not part of the selected update.', status: 'failed' }
      : resourceToDiffState(diffResource)

  const replaceHistorySelection = useCallback((
    nextEntryId: string | null,
    nextPath: string | null = null,
    visibilityId: string | null = null,
  ) => {
    return navigate({
      params,
      replace: true,
      resetScroll: false,
      search: (current) => ({
        ...current,
        entry: nextEntryId ?? undefined,
        path: nextPath ?? undefined,
        visibility_change: visibilityId ?? undefined,
      }),
      to: '/$owner/$repo/history',
    })
  }, [navigate, params])

  const loadOlder = useCallback(async () => {
    const before = loaded.next_cursor
    if (!before || loadingOlder) return
    setLoadingOlder(true)
    setLoadOlderError(null)
    try {
      const page = await loadHistoryPage({
        data: { audience, feed, before, owner: params.owner, repo: params.repo },
      })
      setLoaded((current) => appendHistoryPage(current, page, before))
    } catch (error) {
      setLoadOlderError(error instanceof Error ? error.message : 'Older history is unavailable.')
    } finally {
      setLoadingOlder(false)
    }
  }, [audience, feed, loaded.next_cursor, loadingOlder, params.owner, params.repo])

  const closeDiff = useCallback(
    () => setDiffSelection({ locationKey, dismissed: true }),
    [locationKey],
  )
  const selectAudience = useCallback(
    (nextAudience: ProjectionPreviewAudience) => navigate({
      params,
      replace: true,
      resetScroll: false,
      search: { audience: nextAudience, feed },
      to: '/$owner/$repo/history',
    }),
    [feed, navigate, params],
  )
  const selectFeed = useCallback(
    (nextFeed: 'updates' | 'all') => navigate({
      params, replace: true, resetScroll: false,
      search: { audience, feed: nextFeed },
      to: '/$owner/$repo/history',
    }),
    [audience, navigate, params],
  )
  const selectVisibility = useCallback(
    (change: HistoryVisibilityChange) => {
      setDiffSelection({ locationKey, dismissed: false })
      return replaceHistorySelection(selectedEntryId, change.path, change.id)
    },
    [locationKey, replaceHistorySelection, selectedEntryId],
  )
  const selectEntry = useCallback(
    (entry: HistoryEntrySummary) => {
      setDiffSelection({ locationKey, dismissed: false })
      return replaceHistorySelection(entry.source_id)
    },
    [locationKey, replaceHistorySelection],
  )
  const selectFile = useCallback(
    (file: CommitFile) => {
      setDiffSelection({ locationKey, dismissed: false })
      return replaceHistorySelection(selectedEntryId, file.path)
    },
    [locationKey, replaceHistorySelection, selectedEntryId],
  )
  const saveDiffScroll = useCallback(
    (scrollTop: number) => writeHistoryDiffScroll(diffIdentity, scrollTop),
    [diffIdentity],
  )

  return {
    audience,
    availableAudiences,
    closeDiff,
    entryResource,
    feed,
    selectFeed,
    selectVisibility,
    selectedVisibilityId,
    diffIdentity,
    entries: loaded.entries,
    fileDiffState,
    loadOlder,
    loadOlderError,
    loadingOlder,
    retryDetail: entryResource.retry,
    retryDiff: selectedFilePath && selectedEntry && !selectedFile
      ? undefined
      : diffResource.retry,
    saveDiffScroll,
    selectAudience,
    selectEntry,
    selectFile,
    selectedEntryId,
    selectedFilePath,
    showLoadOlder: loaded.next_cursor !== null,
  }
}
