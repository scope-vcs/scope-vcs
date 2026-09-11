import assert from 'node:assert/strict'
import test from 'node:test'
import type { RequestRevisions } from '@/api/types'
import type { LoadDiscussionsInput } from './request-discussion-api'
import {
  appendDiscussionReferencePage,
  loadDiscussionReferencePage,
  loadMoreRequestDiscussionReferences,
  openRequestDiscussionReferences,
  requestDiscussionReferenceIdentity,
  requestDiscussionReferenceResource,
  selectedDiscussionReferenceQuery,
} from './request-changes-discussion-references'
import { discussion } from './request-discussion-test-fixtures'
import type { RequestDiscussionPage } from './request-discussion-types'

test('endless unique cursors return just the first page and preserve its cursor', async () => {
  let calls = 0
  const page = await loadDiscussionReferencePage(requestInput(), async (input, options) => {
    calls++
    assert.equal(input.limit, 100)
    assert.equal(options.maxResponseBytes, 512 * 1024)
    return discussionPage(['first'], `cursor-${calls}`, 41)
  })
  assert.equal(calls, 1)
  assert.equal(page.next_cursor, 'cursor-1')
  assert.equal(page.snapshot_version, 41)
})

test('slow pages abort transport at the whole-load deadline', async () => {
  let signal: AbortSignal | undefined
  const start = Date.now()
  await assert.rejects(loadDiscussionReferencePage(requestInput(), async (_, options) => {
    signal = options.signal
    return new Promise(() => {})
  }), /timed out/)
  assert.equal(signal?.aborted, true)
  assert.ok(Date.now() - start < 3_000)
})

test('oversized item and byte responses are rejected without another request', async () => {
  await assert.rejects(loadDiscussionReferencePage(requestInput(), async () =>
    discussionPage(Array.from({ length: 101 }, (_, i) => String(i)), 'more', 1)), /page limit/)
  await assert.rejects(loadDiscussionReferencePage(requestInput(), async () =>
    discussionPage(['x'.repeat(512 * 1024)], 'more', 1)), /page limit/)
})

test('continuation preserves snapshot consistency and rejects a repeated cursor', () => {
  const previous = discussionPage(['first'], 'next', 1)
  assert.throws(() => appendDiscussionReferencePage(previous, discussionPage(['later'], null, 2)), /Discussions changed/)
  assert.throws(() => appendDiscussionReferencePage(previous, discussionPage([], 'next', 1)), /repeated a cursor/)
  assert.deepEqual(appendDiscussionReferencePage(previous, discussionPage(['last'], null, 1)).discussions.map(d => d.id), ['first', 'last'])
})

test('many revisions and commits produce only the selected reference query', () => {
  const revisions = {
    review_revision_id: 'revision-19',
    revisions: Array.from({ length: 20 }, (_, i) => ({
      id: `revision-${i}`, position: i, inspection: 'Complete',
      commits: Array.from({ length: 100 }, (_, j) => ({ oid: `commit-${i}-${j}` })),
    })),
  } as RequestRevisions
  const selected = selectedDiscussionReferenceQuery({ ...requestInput(), revision_id: 'revision-3', commit_oid: 'commit-3-7' }, revisions)
  assert.equal(selected?.input.revision_id, 'revision-3')
  assert.equal(selected?.input.commit_oid, 'commit-3-7')
  assert.equal(selected?.input.include_revision_anchor, false)
  const latest = selectedDiscussionReferenceQuery({ owner: 'owner', repo: 'repo', request_id: 'request' }, revisions)
  assert.equal(latest?.input.commit_oid, 'commit-19-99')
  assert.equal(latest?.input.include_revision_anchor, true)
})

function requestInput(): LoadDiscussionsInput {
  return {
    commit_oid: 'a'.repeat(40),
    limit: 100,
    owner: 'owner',
    repo: 'repo',
    request_id: 'request-1',
    revision_id: 'revision-1',
  }
}

function discussionPage(
  ids: string[],
  nextCursor: string | null,
  snapshotVersion: number,
): RequestDiscussionPage {
  return {
    discussions: ids.map((id, index) => discussion(id, index)),
    next_cursor: nextCursor,
    snapshot_version: snapshotVersion,
  }
}

test('a loader page seeds the resource and only a newer snapshot replaces loaded pages', () => {
  const identity = requestDiscussionReferenceIdentity('scope', 'revision-1:commit-a')
  const first = discussionPage(['first'], 'cursor-1', 7)
  assert.equal(openRequestDiscussionReferences(identity, first), first)
  assert.equal(requestDiscussionReferenceResource.peek(identity), first)

  const accumulated = discussionPage(['first', 'second'], null, 7)
  requestDiscussionReferenceResource.write(identity, accumulated)
  assert.equal(openRequestDiscussionReferences(identity, first), accumulated)
  assert.equal(requestDiscussionReferenceResource.peek(identity), accumulated)

  const newer = discussionPage(['fresh'], null, 8)
  assert.equal(openRequestDiscussionReferences(identity, newer), newer)
  assert.equal(requestDiscussionReferenceResource.peek(identity), newer)
})

test('loading more appends the next page under the loaded snapshot', async () => {
  const identity = requestDiscussionReferenceIdentity('scope', 'revision-1:commit-b')
  openRequestDiscussionReferences(identity, discussionPage(['first'], 'cursor-1', 3))
  const cursors: string[] = []
  await loadMoreRequestDiscussionReferences(identity, async (cursor) => {
    cursors.push(cursor)
    return discussionPage(['second'], null, 3)
  })
  assert.deepEqual(cursors, ['cursor-1'])
  const page = requestDiscussionReferenceResource.peek(identity)
  assert.deepEqual(page?.discussions.map(({ id }) => id), ['first', 'second'])
  assert.equal(page?.next_cursor, null)
  await loadMoreRequestDiscussionReferences(identity, async () => assert.fail('no cursor remains'))
})

test('a failed load-more keeps the loaded page and surfaces the error', async () => {
  const identity = requestDiscussionReferenceIdentity('scope', 'revision-1:commit-c')
  const first = discussionPage(['first'], 'cursor-1', 3)
  openRequestDiscussionReferences(identity, first)
  await loadMoreRequestDiscussionReferences(identity, async () => {
    throw new Error('offline')
  })
  const snapshot = requestDiscussionReferenceResource.getSnapshot(identity)
  assert.equal(snapshot.value, first)
  assert.equal((snapshot.error as Error).message, 'offline')
  assert.equal(snapshot.pending, false)
})
