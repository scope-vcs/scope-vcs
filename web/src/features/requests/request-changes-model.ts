import { displayRouteFilePath } from '../../lib/route-file'
import type { CommitDetail, CommitSummary } from '@/api/types'
import type { RequestRevisionListResponse, ViewId } from '@/api/types.generated'
import type { RequestDiscussion } from './request-discussion-types'

type RequestChangeSelection = {
  commit?: string
  revision?: string
}

export function requestChangeSelection(
  revisions: RequestRevisionListResponse['revisions'],
  reviewRevisionId: RequestRevisionListResponse['review_revision_id'],
  search: RequestChangeSelection,
) {
  const selectedRevisionId = search.revision ?? reviewRevisionId
  const revision = selectedRevisionId
    ? revisions.find(({ id }) => id === selectedRevisionId) ?? null
    : null
  const commit = revision && (
    search.commit && revision.commits.some(({ oid }) => oid === search.commit)
      ? search.commit
      : revision.commits.at(-1)?.oid
  ) || null
  const error = selectionError(revision, commit, search, selectedRevisionId)
  return {
    commit,
    error,
    revision,
  }
}

function selectionError(
  revision: RequestRevisionListResponse['revisions'][number] | null,
  commit: string | null,
  search: RequestChangeSelection,
  selectedRevisionId: string | null | undefined,
) {
  if (!revision) {
    return selectedRevisionId
      ? 'This revision or commit is not part of the request.'
      : null
  }
  if (search.commit && commit !== search.commit) {
    return 'This revision or commit is not part of the request.'
  }
  if (commit) return null
  if (revision.inspection === 'Unavailable') {
    return 'This revision could not be inspected within the request review limits.'
  }
  if (revision.inspection === 'Incomplete') {
    return 'This revision’s commit inspection is incomplete.'
  }
  return 'No commits in this revision are visible to you.'
}

export function orderedRequestCommits(
  revisions: RequestRevisionListResponse['revisions'],
): CommitSummary[] {
  return [...revisions].reverse().flatMap((revision) =>
    [...revision.commits].reverse().map((commit) => ({
      author: commit.author,
      change_count: commit.change_count,
      logical_commit_id: commit.oid,
      message: commit.message,
      parent_projected_id: commit.parent_oids[0] ?? null,
      projected_id: requestRevisionCommitId(revision.id, commit.oid),
    })))
}

export function requestRevisionPin(
  revision: RequestRevisionListResponse['revisions'][number] | null,
  commit: string | null,
  pinnedRevisionId: string | undefined,
): RequestChangeSelection | null {
  if (pinnedRevisionId || !revision) return null
  return {
    commit: commit ?? undefined,
    revision: revision.id,
  }
}

export function requestRevisionCommitId(revisionId: string, commitOid: string) {
  return `${revisionId}:${commitOid}`
}

export function requestCommitForListId(
  revisions: RequestRevisionListResponse['revisions'],
  listId: string,
) {
  for (const revision of revisions) {
    const commit = revision.commits.find(({ oid }) =>
      requestRevisionCommitId(revision.id, oid) === listId)
    if (commit) return { commitOid: commit.oid, revision }
  }
  return null
}

export function missingRequestCommitFileError(
  commit: RequestRevisionListResponse['revisions'][number]['commits'][number],
) {
  return commit.files_truncated
    ? 'This file is outside the bounded file list for the selected commit.'
    : 'This file is not part of the selected commit.'
}

export function discussionsForRequestCommit(
  discussions: RequestDiscussion[],
  revision: RequestRevisionListResponse['revisions'][number] | null,
  commitOid: string | null,
) {
  if (!revision || !commitOid) return []
  return discussions
    .filter((discussion) => {
      const anchor = discussion.anchor
      if (!anchor || anchor.revision_id !== revision.id) return false
      if (anchor.commit_oid) return anchor.commit_oid === commitOid
      return commitOid === revision.commits.at(-1)?.oid
    })
    .sort((left, right) => left.opened_position - right.opened_position)
}

export function requestChangesPresentation(
  revisions: RequestRevisionListResponse,
  search: RequestChangeSelection & { comparison?: 'request' | 'push' | 'commit' },
  view: ViewId,
  repoId: string,
  allowRequestComparison: boolean,
) {
  const selection = requestChangeSelection(revisions.revisions, revisions.review_revision_id, search)
  const selectedRevision = selection.revision
  const requestedMode = search.comparison ?? (search.commit ? 'commit' : revisions.comparison?.mode ?? (allowRequestComparison ? 'request' : 'push'))
  const mode = !allowRequestComparison && requestedMode === 'request' ? 'push' : requestedMode
  const comparison = revisions.comparison
  const aggregate = allowRequestComparison && selectedRevision && comparison?.revision_id === selectedRevision.id && comparison.mode === mode
    && (mode !== 'commit' || comparison.head_oid === selection.commit)
    ? comparison : null
  const usesCommitFiles = mode === 'commit' || !allowRequestComparison
  const commits = orderedRequestCommits(selectedRevision ? [selectedRevision] : [])
  const selectedCommitOid = usesCommitFiles ? selection.commit : null
  const selectedCommitId = selectedRevision && selectedCommitOid
    ? requestRevisionCommitId(selectedRevision.id, selectedCommitOid)
    : null
  const commitSummary = selectedRevision?.commits.find(
    ({ oid }) => oid === selectedCommitOid,
  ) ?? null
  const selectedCommitSummary = commitSummary && aggregate
    ? {
        ...commitSummary,
        files: aggregate.files,
        files_truncated: aggregate.inspection !== 'Complete',
      }
    : commitSummary
  const selectedCommit = usesCommitFiles && selectedRevision && selectedCommitSummary
    ? commitDetail(selectedRevision.id, selectedCommitSummary, view, repoId)
    : aggregate ? comparisonDetail(aggregate, view, repoId) : null
  const error = aggregate?.inspection === 'Unavailable'
    ? 'This comparison could not be inspected.'
    : usesCommitFiles || !selectedRevision
      ? selection.error
      : !aggregate ? 'This comparison could not be inspected.' : null
  return {
    aggregate,
    commits: usesCommitFiles ? commits : selectedCommit ? [selectedCommit] : [],
    diffMode: usesCommitFiles ? 'commit' as const : mode,
    error,
    mode,
    selectedCommit,
    selectedCommitId,
    selectedCommitOid,
    selectedCommitSummary,
    selectedRevision,
  }
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
