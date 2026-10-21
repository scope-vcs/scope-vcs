import assert from 'node:assert/strict'
import test from 'node:test'
import type { RequestDiscussionAnchor } from '@/api/types.generated'
import {
  beginRequestAttachmentSubmission,
  finishRequestAttachmentSubmission,
  requestAttachmentDraftKey,
  requestAttachmentSubmissionPayload,
} from './request-attachment-drafts'
import {
  extendLineSelection,
  lineDiscussionAnchor,
  lineDiscussionDraftTarget,
  lineDiscussionSubject,
  settleLineDiscussionDraft,
  lineDiscussionsForFile,
  type LineDiscussionComparison,
} from './request-changes-line-discussions'
import { discussion } from './request-discussion-test-fixtures'

const rendered = { new: new Set([1, 2, 3, 4, 5, 9]), old: new Set([1, 2, 3]) }
const requestLayer: LineDiscussionComparison = { commitOid: null, mode: 'request', path: '/src/lib.rs', revisionId: 'revision-2' }

test('own-revision line discussions sit on their end line only in the matching comparison', () => {
  const discussions = [
    discussion('request-layer', 1, { anchor: anchor({ lines: { side: 'new', start_line: 2, end_line: 4 } }) }),
    discussion('commit', 2, { anchor: anchor({ commit_oid: 'commit-a', lines: { side: 'old', start_line: 3, end_line: 3 } }) }),
    discussion('other-file', 3, { anchor: anchor({ path: '/src/main.rs', lines: { side: 'new', start_line: 1, end_line: 1 } }) }),
    discussion('file-only', 4, { anchor: anchor({}) }),
  ]
  const request = lineDiscussionsForFile(discussions, requestLayer, rendered)
  assert.deepEqual(request.placed.map(({ discussion: { id }, lines }) => [id, lines]), [
    ['request-layer', { side: 'new', start_line: 2, end_line: 4 }],
  ])
  assert.deepEqual(request.fileLevel, [])

  const commit = lineDiscussionsForFile(discussions, { ...requestLayer, commitOid: 'commit-a', mode: 'commit' }, rendered)
  assert.deepEqual(commit.placed.map(({ discussion: { id }, lines }) => [id, lines]), [
    ['commit', { side: 'old', start_line: 3, end_line: 3 }],
  ])
  assert.deepEqual(lineDiscussionsForFile(discussions, { ...requestLayer, mode: 'push' }, rendered), { fileLevel: [], placed: [] })
})

test('carried-forward discussions use the mapped latest range and list outdated ones at file level', () => {
  const lines = { side: 'new' as const, start_line: 1, end_line: 2 }
  const discussions = [
    discussion('outdated', 1, { anchor: anchor({ revision_id: 'revision-1', lines, latest: { kind: 'outdated', revision_id: 'revision-2' } }) }),
    discussion('mapped', 2, { anchor: anchor({ revision_id: 'revision-1', lines, latest: { kind: 'mapped', revision_id: 'revision-2', start_line: 8, end_line: 9 } }) }),
    discussion('stale-latest', 3, { anchor: anchor({ revision_id: 'revision-0', lines, latest: { kind: 'mapped', revision_id: 'revision-3', start_line: 1, end_line: 1 } }) }),
  ]
  const latest = lineDiscussionsForFile(discussions, requestLayer, rendered)
  assert.deepEqual(latest.placed.map(({ discussion: { id }, lines }) => [id, lines]), [
    ['mapped', { side: 'new', start_line: 8, end_line: 9 }],
  ])
  assert.deepEqual(latest.fileLevel.map(({ discussion: { id }, lines }) => [id, lines]), [['outdated', null]])
  const commit = lineDiscussionsForFile(discussions, { ...requestLayer, commitOid: 'commit-a', mode: 'commit' }, rendered)
  assert.deepEqual(commit, { fileLevel: [], placed: [] })
})

test('line discussions whose end line is not rendered are listed at file level with their range', () => {
  const hidden = { side: 'old' as const, start_line: 6, end_line: 7 }
  const discussions = [discussion('hidden', 1, { anchor: anchor({ lines: hidden }) })]
  assert.deepEqual(
    lineDiscussionsForFile(discussions, requestLayer, rendered).fileLevel.map(({ lines }) => lines),
    [hidden],
  )
  assert.deepEqual(
    lineDiscussionsForFile(discussions, requestLayer, null).fileLevel.map(({ lines }) => lines),
    [hidden],
  )
})

test('shift selection extends a range on one side and restarts on the other side', () => {
  const first = extendLineSelection(null, { line: 7, side: 'new' }, true)
  assert.deepEqual(first, { side: 'new', start_line: 7, end_line: 7 })
  const extended = extendLineSelection(first, { line: 4, side: 'new' }, true)
  assert.deepEqual(extended, { side: 'new', start_line: 4, end_line: 7 })
  assert.deepEqual(extendLineSelection(extended, { line: 5, side: 'old' }, true), { side: 'old', start_line: 5, end_line: 5 })
  assert.deepEqual(extendLineSelection(extended, { line: 9, side: 'new' }, false), { side: 'new', start_line: 9, end_line: 9 })
})

test('new line discussions anchor to the selected commit only in commit mode', () => {
  const lines = { side: 'new' as const, start_line: 3, end_line: 4 }
  assert.deepEqual(lineDiscussionAnchor({ ...requestLayer, commitOid: 'ignored' }, lines), {
    commit_oid: null,
    lines,
    path: '/src/lib.rs',
    revision_id: 'revision-2',
  })
  assert.equal(lineDiscussionAnchor({ ...requestLayer, commitOid: 'commit-a', mode: 'commit' }, lines).commit_oid, 'commit-a')
})

function anchor(overrides: Partial<RequestDiscussionAnchor>): RequestDiscussionAnchor {
  return {
    commit_oid: null,
    latest: null,
    lines: null,
    path: '/src/lib.rs',
    revision_id: 'revision-2',
    revision_position: 2,
    ...overrides,
  }
}

test('a line composer keeps one draft per file while each submitted range gets its own id', () => {
  const draftKey = (comparison: LineDiscussionComparison) => requestAttachmentDraftKey({
    accessScope: 'scope',
    repoId: 'repo',
    requestId: 'request',
    target: lineDiscussionDraftTarget(comparison),
    viewerId: 'viewer',
  })
  const key = draftKey(requestLayer)
  assert.equal(key, draftKey({ ...requestLayer }))
  assert.notEqual(key, draftKey({ ...requestLayer, path: '/src/other.rs' }))
  assert.notEqual(key, draftKey({ ...requestLayer, revisionId: 'revision-1' }))
  const submit = (lines: { side: 'old' | 'new'; start_line: number; end_line: number }) => {
    const id = beginRequestAttachmentSubmission(key, requestAttachmentSubmissionPayload({
      action: 'primary', baseText: null, markdown: 'Same body', quoteId: null, subject: lineDiscussionSubject(lines),
    }))
    assert.ok(id)
    finishRequestAttachmentSubmission(id, false)
    return id
  }
  const firstId = submit({ side: 'new', start_line: 3, end_line: 3 })
  assert.equal(submit({ side: 'new', start_line: 3, end_line: 3 }), firstId)
  for (const lines of [
    { side: 'new', start_line: 4, end_line: 4 },
    { side: 'old', start_line: 3, end_line: 3 },
    { side: 'new', start_line: 2, end_line: 3 },
  ] as const) {
    assert.notEqual(submit(lines), firstId)
  }
})

test('a finished post settles only the composer it was submitted from', () => {
  const lineA = { side: 'new', start_line: 3, end_line: 3 } as const
  const lineB = { side: 'new', start_line: 8, end_line: 8 } as const
  const draftOn = (key: string, lines: typeof lineA | typeof lineB) => ({ error: null, key, lines })
  const submittedA = { key: 'layer', lines: lineA }

  const openB = draftOn('layer', lineB)
  assert.equal(settleLineDiscussionDraft(openB, submittedA, { posted: true }), openB)
  assert.equal(settleLineDiscussionDraft(openB, submittedA, { error: 'Lost' }), openB)
  const otherLayerA = draftOn('other-layer', lineA)
  assert.equal(settleLineDiscussionDraft(otherLayerA, submittedA, { posted: true }), otherLayerA)

  assert.equal(settleLineDiscussionDraft(draftOn('layer', lineA), submittedA, { posted: true }), null)
  assert.deepEqual(
    settleLineDiscussionDraft(draftOn('layer', lineA), submittedA, { error: 'Lost' }),
    { ...draftOn('layer', lineA), error: 'Lost' },
  )
})
