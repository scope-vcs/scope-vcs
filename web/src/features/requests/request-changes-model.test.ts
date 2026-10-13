import assert from 'node:assert/strict'
import test from 'node:test'
import { discussion } from './request-discussion-test-fixtures'
import {
  discussionsForRequestCommit,
  missingRequestCommitFileError,
  orderedRequestCommits,
  requestCommitForListId,
  requestChangeSelection,
  requestRevisionPin,
  requestChangesPresentation,
} from './request-changes-model'
import type { RequestRevisionListResponse } from '@/api/types.generated'

const revisions: RequestRevisionListResponse['revisions'] = [
  revision('revision-1', 1, ['commit-a', 'commit-b']),
  revision('revision-2', 4, ['commit-c']),
]

test('defaults to the newest revision and its newest commit', () => {
  const selection = requestChangeSelection(revisions, 'revision-2', {})
  assert.equal(selection.revision?.id, 'revision-2')
  assert.equal(selection.commit, 'commit-c')
  assert.equal(selection.error, null)
})

test('keeps the newest revision selected when its inspection is unavailable', () => {
  const hiddenLatest = revision('revision-3', 6, [], 'Unavailable')
  const selection = requestChangeSelection(
    [...revisions, hiddenLatest],
    'revision-3',
    {},
  )
  assert.equal(selection.revision?.id, 'revision-3')
  assert.equal(selection.commit, null)
  assert.match(selection.error ?? '', /could not be inspected/)
})

test('an explicit revision remains pinned when a newer revision arrives', () => {
  const newer = revision('revision-3', 6, ['commit-d'])
  const selection = requestChangeSelection(
    [...revisions, newer],
    'revision-3',
    { revision: 'revision-2' },
  )
  assert.equal(selection.revision?.id, 'revision-2')
  assert.equal(selection.commit, 'commit-c')
})

test('initial selection becomes an explicit revision pin before refreshes', () => {
  const initial = requestChangeSelection(revisions, 'revision-2', {})
  assert.deepEqual(
    requestRevisionPin(initial.revision, initial.commit, undefined),
    { commit: 'commit-c', revision: 'revision-2' },
  )
  assert.equal(
    requestRevisionPin(initial.revision, initial.commit, 'revision-2'),
    null,
  )
})

test('keeps revision and commit selection consistent', () => {
  assert.deepEqual(
    requestChangeSelection(revisions, 'revision-2', {
      commit: 'commit-a',
      revision: 'revision-1',
    }),
    {
      commit: 'commit-a',
      revision: revisions[0],
      error: null,
    },
  )
  assert.match(
    requestChangeSelection(revisions, 'revision-2', {
      commit: 'commit-c',
      revision: 'revision-1',
    }).error ?? '',
    /not part of the request/,
  )
})

test('keeps a valid commit selectable when its file list is truncated', () => {
  const oversized = [revision('revision-large', 1, ['commit-large'])]
  oversized[0].commits[0].files = []
  oversized[0].commits[0].change_count = 10_001
  oversized[0].commits[0].files_truncated = true

  const selection = requestChangeSelection(
    oversized,
    'revision-large',
    { commit: 'commit-large', revision: 'revision-large' },
  )
  assert.equal(selection.commit, 'commit-large')
  assert.equal(selection.error, null)
  assert.match(
    missingRequestCommitFileError(oversized[0].commits[0]),
    /outside the bounded file list/,
  )
})

test('lists newest request commits first', () => {
  assert.deepEqual(
    orderedRequestCommits(revisions).map(({ projected_id }) => projected_id),
    [
      'revision-2:commit-c',
      'revision-1:commit-b',
      'revision-1:commit-a',
    ],
  )
})

test('keeps repeated commit OIDs distinct across revisions', () => {
  const repeated = [
    revision('revision-1', 1, ['commit-a']),
    revision('revision-2', 2, ['commit-a']),
  ]
  assert.deepEqual(
    orderedRequestCommits(repeated).map(({ projected_id }) => projected_id),
    ['revision-2:commit-a', 'revision-1:commit-a'],
  )
  assert.equal(
    requestCommitForListId(repeated, 'revision-2:commit-a')?.revision.id,
    'revision-2',
  )
})

test('orders matching commit and revision discussions chronologically', () => {
  const discussions = [
    discussion('later', 8, { anchor: { revision_id: 'revision-1', revision_position: 1, commit_oid: 'commit-b', path: null } }),
    discussion('revision', 5, { anchor: { revision_id: 'revision-1', revision_position: 1, commit_oid: null, path: null } }),
    discussion('other', 6, { anchor: { revision_id: 'revision-1', revision_position: 1, commit_oid: 'commit-a', path: null } }),
  ]
  assert.deepEqual(
    discussionsForRequestCommit(discussions, revisions[0], 'commit-b')
      .map(({ id }) => id),
    ['revision', 'later'],
  )
})

for (const view of ['public', 'restricted']) {
  test(`${view} request and push comparisons show only the selected child layer at its parent pin`, () => {
    const child = revision('child-revision', 2, ['parent-commit', 'child-commit'])
    child.base = { kind: 'request', request_id: 'parent', revision_id: 'parent-revision', head_oid: 'parent-commit' }
    for (const mode of ['request', 'push'] as const) {
      const response: RequestRevisionListResponse = {
        comparison: {
          revision_id: child.id,
          mode,
          base_oid: 'parent-commit',
          head_oid: 'child-commit',
          inspection: 'Complete',
          files: child.commits[1].files,
        },
        has_earlier_revisions: false,
        review_revision_id: child.id,
        revisions: [child],
      }
      const model = requestChangesPresentation(response, { revision: child.id, comparison: mode }, view, 'repo')
      assert.equal(model.error, null)
      assert.equal(model.mode, mode)
      assert.equal(model.diffMode, mode)
      assert.equal(model.selectedCommitOid, null)
      assert.equal(model.selectedCommit?.logical_commit_id, 'child-commit')
      assert.deepEqual(model.selectedCommit?.files.map(({ path }) => path), ['/child-commit.txt'])
      const unavailable = requestChangesPresentation({ ...response, comparison: null }, { comparison: mode }, view, 'repo')
      assert.equal(unavailable.selectedCommit, null)
      assert.match(unavailable.error ?? '', /could not be inspected/)
    }
  })
}

test('private commit comparison uses inspected files while retaining the bounded commit metadata', () => {
  const bounded = revision('revision-budget', 1, ['commit-selected'])
  const commit = bounded.commits[0]
  const inspectedFiles = commit.files
  commit.files = []
  commit.change_count = 501
  commit.files_truncated = true
  const response: RequestRevisionListResponse = {
    comparison: {
      revision_id: bounded.id,
      mode: 'commit',
      base_oid: 'base',
      head_oid: commit.oid,
      files: inspectedFiles,
      inspection: 'Incomplete',
    },
    has_earlier_revisions: false,
    review_revision_id: bounded.id,
    revisions: [bounded],
  }
  const model = requestChangesPresentation(response, { comparison: 'commit', commit: commit.oid }, 'private', 'repo')
  assert.equal(model.error, null)
  assert.deepEqual(model.selectedCommit?.files.map(({ path }) => path), ['/commit-selected.txt'])
  assert.deepEqual(model.selectedCommitSummary?.files.map(({ path }) => path), ['commit-selected.txt'])
  assert.equal(model.selectedCommit?.change_count, 501)
  assert.equal(model.selectedCommit?.files_truncated, true)
  assert.equal(model.selectedCommit?.author, commit.author)
  assert.equal(model.selectedCommit?.message, commit.message)
  assert.equal(model.selectedCommit?.logical_commit_id, commit.oid)
})

function revision(
  id: string,
  position: number,
  commits: string[],
  inspection: RequestRevisionListResponse['revisions'][number]['inspection'] = 'Complete',
): RequestRevisionListResponse['revisions'][number] {
  return {
    base: { kind: 'main' as const, review_base_oid: 'base' },
    base_version: 1,
    actor: { handle: 'adam', id: 'user-1' },
    commits: commits.map((oid) => ({
      author: 'Adam <adam@example.com>',
      authored_at_unix: position,
      change_count: 1,
      files: [{
        kind: 'Modified' as const,
        new_mode: '100644',
        new_oid: oid,
        old_mode: '100644',
        old_oid: 'base',
        path: `${oid}.txt`,
        label: 'public' as const,
      }],
      message: `Commit ${oid}`,
      oid,
      parent_oids: ['base'],
      files_truncated: false,
    })),
    created_at_unix: position,
    id,
    inspection,
    new_head_oid: commits.at(-1) ?? 'base',
    old_head_oid: 'base',
    position,
  }
}
