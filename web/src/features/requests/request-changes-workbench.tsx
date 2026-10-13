import type { CommitDetail, CommitSummary, ReviewFileDiff } from '@/api/types'
import type {
  CommitFileResponse,
  ViewId,
  RequestRevisionListResponse,
} from '@/api/types.generated'
import type { LoadRequestRevisionDiffInput } from '@/api/requests'
import { PendingSurface } from '@/components/pending-surface'
import { BlockSkeleton, TextSkeleton } from '@/components/ui/skeleton'
import {
  historyDiffCacheKey,
  historyDiffResource,
} from '@/features/history/history-resource-cache'
import { HistoryWorkbench } from '@/features/history/history-workbench'
import {
  resourceToDiffState,
  type CommitDetailState,
  type CommitFileDiffState,
} from '@/features/history/history-state'
import { shortOid } from '@/lib/short-oid'
import { useCachedResource } from '@/lib/use-cached-resource'
import { AbsoluteTimestamp } from '@/components/timestamp'
import { commitCount } from './request-revision-stepper'
import { Link } from '@tanstack/react-router'
import { MessageSquare } from 'lucide-react'
import { displayRouteFilePath } from '@/lib/route-file'
import { useCallback, useMemo } from 'react'
import { compactDiscussionSummary } from './discussion-preview-text'
import {
  discussionReferenceQuery,
  loadMoreRequestDiscussionReferences,
  openRequestDiscussionReferences,
  requestDiscussionReferenceIdentity,
  requestDiscussionReferenceResource,
} from './request-changes-discussion-references'
import type { LoadDiscussionsInput } from './request-discussion-api'
import {
  discussionsForRequestCommit,
  missingRequestCommitFileError,
  orderedRequestCommits,
  requestCommitForListId,
  requestChangeSelection,
  requestRevisionCommitId,
} from './request-changes-model'
import type {
  RequestDiscussion,
  RequestDiscussionPage,
} from './request-discussion-types'
import { actorHandle } from './request-actor'

export type RequestChangesSearch = {
  comparison?: 'request' | 'push' | 'commit'
  commit?: string
  path?: string
  revision?: string
}

export type RequestChangesDiscussionReferences = {
  commitKey: string | null
  page: RequestDiscussionPage | null
}

type DiscussionReferenceState = {
  discussions: RequestDiscussion[]
  error: string | null
  incomplete: boolean
  loadMore?: () => void
  loading: boolean
  retry?: () => void
}

export function RequestChangesWorkbench({
  accessScope,
  allowRequestComparison = true,
  view,
  initialDiscussionReferences,
  loadDiff,
  loadDiscussions,
  onSearchChange,
  params,
  repoId,
  revisions,
  search,
}: {
  accessScope: string | null
  allowRequestComparison?: boolean
  view: ViewId
  initialDiscussionReferences: RequestChangesDiscussionReferences
  loadDiff: (
    input: LoadRequestRevisionDiffInput,
    signal?: AbortSignal,
  ) => Promise<ReviewFileDiff>
  loadDiscussions: (input: LoadDiscussionsInput) => Promise<RequestDiscussionPage>
  onSearchChange: (search: RequestChangesSearch) => void
  params: { owner: string; repo: string; request_id: string }
  repoId: string
  revisions: RequestRevisionListResponse
  search: RequestChangesSearch
}) {
  const model = useRequestChangesModel({
    accessScope,
    view,
    loadDiff,
    onSearchChange,
    params,
    repoId,
    revisions,
    search,
  })
  const discussionReferences = useRequestDiscussionReferences({
    accessScope,
    commitOid: model.selectedCommitOid,
    initialReferences: initialDiscussionReferences,
    loadDiscussions,
    params,
    revision: model.selectedRevision,
  })
  const references = useMemo(
    () => discussionsForRequestCommit(
      discussionReferences.discussions,
      model.selectedRevision,
      model.selectedCommitOid,
    ),
    [discussionReferences.discussions, model.selectedCommitOid, model.selectedRevision],
  )
  const commitContext = useMemo(
    () => model.selectedRevision ? (
      <RequestCommitContext
        commit={model.selectedCommitSummary}
        discussions={references}
        discussionReferences={discussionReferences}
        hasEarlierRevisions={revisions.has_earlier_revisions}
        params={params}
        revision={model.selectedRevision}
      />
    ) : undefined,
    [discussionReferences, model.selectedCommitSummary, model.selectedRevision, params, references, revisions.has_earlier_revisions],
  )
  const emptyDescription = model.commitState.status === 'failed'
    ? model.commitState.error ?? 'Request changes are unavailable.'
    : 'Changes appear here after the request branch is pushed.'
  const emptyTitle = model.commitState.status === 'failed'
    ? 'Request changes unavailable'
    : 'No request changes yet'

  return (
    <>
      <nav aria-label="Comparison" className="flex flex-wrap gap-2 border-b border-border px-5 py-2 text-xs sm:px-6 lg:px-8">
        {(['request', 'push', 'commit'] as const).filter((mode) => mode !== 'request' || allowRequestComparison).map((mode) => (
          <button
            aria-current={model.mode === mode ? 'page' : undefined}
            className="rounded px-2 py-1 text-muted-foreground hover:bg-muted hover:text-foreground aria-[current]:bg-muted aria-[current]:text-foreground"
            disabled={mode === 'commit' && !model.selectedRevision?.commits.length}
            key={mode}
            onClick={() => onSearchChange({ revision: model.selectedRevision?.id, comparison: mode, commit: mode === 'commit' ? model.selectedRevision?.commits.at(-1)?.oid : undefined })}
            type="button"
          >
            {mode === 'request' ? 'Request layer' : mode === 'push' ? 'Push changes' : 'Commits'}
          </button>
        ))}
      </nav>
      <HistoryWorkbench
        commitContext={commitContext}
        commitState={model.commitState}
        commits={model.commits}
        diffIdentity={model.diffIdentity}
        emptyDescription={emptyDescription}
        emptyTitle={emptyTitle}
        fileDiffState={model.fileDiffState}
        header={model.selectedRevision ? (
          <>
            <RequestRevisionHeader revision={model.selectedRevision} />
            {model.mode !== 'commit' && revisions.comparison?.inspection === 'Incomplete' ? (
              <p role="status" className="border-b border-border px-5 py-2 text-xs text-muted-foreground sm:px-6 lg:px-8">
                This comparison’s file inspection is incomplete. Showing the available files.
              </p>
            ) : null}
          </>
        ) : undefined}
        onCloseDiff={model.closeDiff}
        onRetryDiff={model.retryDiff}
        onSelectCommit={model.selectCommit}
        onSelectFile={model.selectFile}
        selectedCommitId={model.selectedCommitId}
        selectedFilePath={model.selectedFilePath}
      />
    </>
  )
}

function useRequestDiscussionReferences({
  accessScope,
  commitOid,
  initialReferences,
  loadDiscussions,
  params,
  revision,
}: {
  accessScope: string | null
  commitOid: string | null
  initialReferences: RequestChangesDiscussionReferences
  loadDiscussions: (input: LoadDiscussionsInput) => Promise<RequestDiscussionPage>
  params: { owner: string; repo: string; request_id: string }
  revision: RequestRevisionListResponse['revisions'][number] | null
}): DiscussionReferenceState {
  const query = revision && commitOid
    ? discussionReferenceQuery(params, revision, commitOid)
    : null
  const identity = query && accessScope
    ? requestDiscussionReferenceIdentity(accessScope, params.request_id, query.key)
    : null
  const seed = query && query.key === initialReferences.commitKey ? initialReferences.page : null
  useMemo(() => {
    if (identity && seed) openRequestDiscussionReferences(identity, seed)
  }, [identity, seed])
  const input = query?.input
  const loadFirstPage = useCallback(() => {
    if (!input) throw new Error('Select a commit.')
    return loadDiscussions(input)
  }, [input, loadDiscussions])
  const page = useCachedResource({
    fallbackError: 'Discussion references are unavailable.',
    identity,
    load: loadFirstPage,
    resource: requestDiscussionReferenceResource,
  })
  const loadMore = useCallback(() => {
    if (!identity || !input) return
    void loadMoreRequestDiscussionReferences(
      identity,
      (cursor) => loadDiscussions({ ...input, cursor }),
    )
  }, [identity, input, loadDiscussions])
  const retry = page.retry
  return {
    discussions: page.value?.discussions ?? [],
    error: page.error,
    incomplete: page.value?.next_cursor != null,
    loadMore: page.status === 'loaded' && page.value.next_cursor && !page.refreshing && !page.error
      ? loadMore
      : undefined,
    loading: page.status === 'loading' || page.refreshing,
    retry: page.status === 'failed' || page.status === 'loaded' && page.error
      ? retry
      : undefined,
  }
}

function useRequestChangesModel({
  accessScope,
  view,
  loadDiff,
  onSearchChange,
  params,
  repoId,
  revisions,
  search,
}: {
  accessScope: string | null
  view: ViewId
  loadDiff: (
    input: LoadRequestRevisionDiffInput,
    signal?: AbortSignal,
  ) => Promise<ReviewFileDiff>
  onSearchChange: (search: RequestChangesSearch) => void
  params: { owner: string; repo: string; request_id: string }
  repoId: string
  revisions: RequestRevisionListResponse
  search: RequestChangesSearch
}) {
  const orderedRevisions = revisions.revisions
  const selection = useMemo(
    () => requestChangeSelection(
      orderedRevisions,
      revisions.review_revision_id,
      search,
    ),
    [orderedRevisions, revisions.review_revision_id, search],
  )
  const selectedRevision = selection.revision
  const mode = search.comparison ?? (search.commit ? 'commit' : revisions.comparison?.mode ?? 'request')
  const comparison = revisions.comparison
  const aggregate = mode !== 'commit' && selectedRevision && comparison?.revision_id === selectedRevision.id && comparison.mode === mode
    ? comparison : null
  const commits = useMemo(
    () => orderedRequestCommits(selectedRevision ? [selectedRevision] : []),
    [selectedRevision],
  )
  const selectedRevisionId = selectedRevision?.id ?? null
  const selectedCommitOid = mode === 'commit' ? selection.commit : null
  const selectedCommitId = selectedRevision && selectedCommitOid
    ? requestRevisionCommitId(selectedRevision.id, selectedCommitOid)
    : null
  const generation = mode === 'commit' ? `${selectedRevision?.position ?? 0}` : `${aggregate?.base_oid ?? ''}:${aggregate?.head_oid ?? ''}`
  const revisionKey = `request:${params.request_id}:${selectedRevision?.id ?? 'none'}:${mode}`
  const selectedCommitSummary = selectedRevision?.commits.find(
    ({ oid }) => oid === selectedCommitOid,
  ) ?? null
  const selectedCommit = aggregate
    ? comparisonDetail(aggregate, view, repoId)
    : mode === 'commit' && selectedRevision && selectedCommitSummary
    ? commitDetail(
        selectedRevision.id,
        selectedCommitSummary,
        view,
        repoId,
      )
    : null
  const selectedFilePath = search.path ?? null
  const selectedFile = selectedCommit?.files.find(
    ({ path }) => path === selectedFilePath,
  ) ?? null
  const diffIdentity = accessScope && selectedCommit && selectedFile && selectedRevision
    ? historyDiffCacheKey({
        scope: accessScope,
        view,
        commit: selectedCommit.projected_id,
        generation,
        newOid: selectedFile.new_oid,
        oldOid: selectedFile.old_oid,
        path: selectedFile.path,
        repoId,
        revisionKey,
      })
    : null
  const loadSelectedDiff = useCallback(
    async (signal: AbortSignal) => {
      if (!selectedRevisionId || !selectedFilePath || mode === 'commit' && !selectedCommitOid) {
        throw new Error('Select a changed file.')
      }
      return loadDiff({
        ...params,
        comparison: mode,
        commit_oid: selectedCommitOid ?? undefined,
        path: selectedFilePath,
        revision_id: selectedRevisionId,
      }, signal)
    },
    [loadDiff, params, selectedCommitOid, selectedFilePath, selectedRevisionId, mode],
  )
  const diffResource = useCachedResource({
    fallbackError: 'Request file diff is unavailable.',
    identity: diffIdentity,
    load: loadSelectedDiff,
    resource: historyDiffResource,
  })
  const inspectionError = mode !== 'commit'
    ? !selectedRevision ? selection.error : !aggregate || aggregate.inspection === 'Unavailable' ? 'This comparison could not be inspected.' : null
    : selection.error
  const commitState: CommitDetailState = inspectionError
    ? { commit: null, error: inspectionError, status: 'failed' }
    : selectedCommit
      ? { commit: selectedCommit, error: null, status: 'loaded' }
      : { commit: null, error: null, status: 'idle' }
  const fileDiffState: CommitFileDiffState =
    selectedFilePath && selectedCommit && !selectedFile
      ? {
          diff: null,
          error: selectedCommitSummary
            ? missingRequestCommitFileError(selectedCommitSummary)
            : 'This file is not part of the selected commit.',
          status: 'failed',
        }
      : resourceToDiffState(diffResource)

  function replaceSelection(
    revision: typeof selectedRevision,
    commitOid: string | null,
    path: string | null,
  ) {
    onSearchChange({
      comparison: mode,
      commit: commitOid ?? undefined,
      path: path ?? undefined,
      revision: revision?.id,
    })
  }

  return {
    closeDiff: () => replaceSelection(selectedRevision, selectedCommitOid, null),
    commits: aggregate && selectedCommit ? [selectedCommit] : mode === 'commit' ? commits : [],
    commitState,
    mode,
    diffIdentity,
    fileDiffState,
    retryDiff: selectedFilePath && selectedCommit && !selectedFile
      ? undefined
      : diffResource.retry,
    selectCommit: (commit: CommitSummary) => {
      const selected = requestCommitForListId(orderedRevisions, commit.projected_id)
      replaceSelection(selected?.revision ?? null, selected?.commitOid ?? null, null)
    },
    selectFile: (file: CommitFileResponse) =>
      replaceSelection(selectedRevision, selectedCommitOid, file.path),
    selectedCommitId: selectedCommit?.projected_id ?? selectedCommitId,
    selectedCommitOid,
    selectedCommitSummary,
    selectedFilePath,
    selectedRevision,
  }
}

function RequestRevisionHeader({ revision }: { revision: RequestRevisionListResponse['revisions'][number] }) {
  return (
    <header className="border-b border-border px-(--app-inset) py-4">
      <h1 className="text-lg font-semibold leading-6 tracking-[-0.01em]">Revision {revision.position}</h1>
      <p className="mt-1.5 flex flex-wrap items-center gap-x-2 gap-y-1 text-xs text-muted-foreground">
        <span>{actorHandle(revision.actor)} pushed {commitCount(revision)}</span>
        <span aria-hidden="true">·</span>
        <AbsoluteTimestamp value={revision.created_at_unix} />
        {revision.old_head_oid && revision.new_head_oid ? (
          <>
            <span aria-hidden="true">·</span>
            <span className="font-mono text-xs">
              {shortOid(revision.old_head_oid)} → {shortOid(revision.new_head_oid)}
            </span>
          </>
        ) : null}
      </p>
    </header>
  )
}

function RequestCommitContext({
  commit,
  discussionReferences,
  discussions,
  hasEarlierRevisions,
  params,
  revision,
}: {
  commit: RequestRevisionListResponse['revisions'][number]['commits'][number] | null
  discussionReferences: DiscussionReferenceState
  discussions: RequestDiscussion[]
  hasEarlierRevisions: boolean
  params: { owner: string; repo: string; request_id: string }
  revision: RequestRevisionListResponse['revisions'][number]
}) {
  return (
    <div className="mt-3 border-t border-border pt-3 text-xs text-muted-foreground empty:hidden">
      {revision.inspection !== 'Complete' || hasEarlierRevisions ? (
        <p>
          {[
            revision.inspection === 'Incomplete'
              ? 'Commit inspection for this revision is incomplete.'
              : null,
            revision.inspection === 'Unavailable'
              ? 'Commit inspection for this revision is unavailable.'
              : null,
            hasEarlierRevisions ? 'Earlier request revisions are omitted.' : null,
          ].filter(Boolean).join(' ')}
        </p>
      ) : null}
      {commit?.files_truncated ? (
        <p className="mt-2">
          Showing {commit.files.length} of {commit.change_count} changed files because the file list is bounded.
        </p>
      ) : null}
      {discussions.length > 0 ? (
        <div
          aria-label="Related discussions"
          className="scope-content-enter mt-3 space-y-2"
        >
          {discussions.map((discussion) => (
            <Link
              className="flex min-w-0 items-center gap-2 text-foreground underline-offset-4 hover:underline"
              hash={`discussion-${discussion.id}`}
              key={discussion.id}
              params={{
                owner: params.owner,
                repo: params.repo,
                requestId: params.request_id,
              }}
              search={{ discussion: discussion.id }}
              to="/$owner/$repo/requests/$requestId"
            >
              <MessageSquare className="size-3.5 shrink-0" />
              <span className="truncate">{compactDiscussionSummary(discussion.body_markdown)}</span>
              <span className="ml-auto shrink-0 text-muted-foreground">
                {discussion.status === 'Resolved' ? 'Resolved' : 'Open'}
              </span>
            </Link>
          ))}
        </div>
      ) : null}
      {discussionReferences.loading ? (
        <PendingSurface
          className="mt-3 min-h-6"
          label="Loading discussion references"
        >
          <div className="space-y-2">
            <div className="flex items-center gap-2">
              <BlockSkeleton className="size-3.5" />
              <TextSkeleton length="long" size="meta" />
              <TextSkeleton className="ml-auto" length="tiny" size="meta" />
            </div>
            <div className="flex items-center gap-2">
              <BlockSkeleton className="size-3.5" />
              <TextSkeleton length="medium" size="meta" />
              <TextSkeleton className="ml-auto" length="tiny" size="meta" />
            </div>
          </div>
        </PendingSurface>
      ) : null}
      {discussionReferences.error ? (
        <div className="mt-3 flex items-center gap-2">
          <span>{discussionReferences.error}</span>
          <button
            className="font-medium text-foreground underline-offset-4 hover:underline"
            onClick={discussionReferences.retry}
            type="button"
          >
            Retry
          </button>
        </div>
      ) : null}
      {discussionReferences.incomplete ? (
        <div className="mt-3 flex flex-wrap items-center gap-2">
          <span>More discussions available</span>
          {discussionReferences.loadMore ? (
            <button
              className="font-medium text-foreground underline-offset-4 hover:underline"
              onClick={discussionReferences.loadMore}
              type="button"
            >
              Load more
            </button>
          ) : null}
        </div>
      ) : null}
    </div>
  )
}

function commitDetail(
  revisionId: string,
  commit: RequestRevisionListResponse['revisions'][number]['commits'][number],
  view: ViewId,
  repoId: string,
): CommitDetail {
  return {
    view,
    author: commit.author,
    change_count: commit.change_count,
    files_truncated: commit.files_truncated,
    files: commit.files.map((file) => ({
      ...file,
      path: `/${displayRouteFilePath(file.path)}`,
    })),
    logical_commit_id: commit.oid,
    message: commit.message,
    parent_projected_id: commit.parent_oids[0] ?? null,
    projected_id: requestRevisionCommitId(revisionId, commit.oid),
    repo_id: repoId,
  }
}

function comparisonDetail(
  comparison: NonNullable<RequestRevisionListResponse['comparison']>,
  view: ViewId,
  repoId: string,
): CommitDetail {
  return {
    view,
    repo_id: repoId,
    projected_id: `${comparison.revision_id}:${comparison.mode}:${comparison.base_oid}:${comparison.head_oid}`,
    logical_commit_id: comparison.head_oid ?? '',
    parent_projected_id: comparison.base_oid,
    author: null,
    message: comparison.mode === 'request' ? 'Request layer' : 'Changes in this push',
    change_count: comparison.files.length,
    files_truncated: comparison.inspection !== 'Complete',
    files: comparison.files.map((file) => ({ ...file, path: `/${displayRouteFilePath(file.path)}` })),
  }
}
