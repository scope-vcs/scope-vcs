import assert from 'node:assert/strict'
import test from 'node:test'
import type { LoadDiscussionsInput } from '../discussion/request-discussion-api'
import {
  appendDiscussionReferencePage,
  loadDiscussionReferencePage,
  loadMoreRequestDiscussionReferences,
  requestDiscussionReferenceIdentity,
  requestDiscussionReferenceResource,
} from './request-changes-discussion-references'
import { discussion } from '../discussion/request-discussion-test-fixtures'
import type { RequestDiscussionPage } from '../discussion/request-discussion-types'

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

test('loading more appends the next page under the loaded snapshot', async () => {
  const identity = requestDiscussionReferenceIdentity('scope', 'request-1', 'revision-1:commit-b')
  requestDiscussionReferenceResource.write(identity, discussionPage(['first'], 'cursor-1', 3))
  const cursors: string[] = []
  await loadMoreRequestDiscussionReferences(identity, async (cursor) => {
    cursors.push(cursor)
    return discussionPage(['second'], null, 3)
  })
  assert.deepEqual(cursors, ['cursor-1'])
  const page = requestDiscussionReferenceResource.peek(identity)
  assert.deepEqual(page?.discussions.map(({ id }) => id), ['first', 'second'])
  assert.equal(page.next_cursor, null)
  await loadMoreRequestDiscussionReferences(identity, async (cursor) => {
    cursors.push(cursor)
    return discussionPage(['unexpected'], null, 3)
  })
  assert.deepEqual(cursors, ['cursor-1'])
  assert.equal(requestDiscussionReferenceResource.peek(identity), page)
})

test('a failed load-more keeps the loaded page and surfaces the error', async () => {
  const identity = requestDiscussionReferenceIdentity('scope', 'request-1', 'revision-1:commit-c')
  const first = discussionPage(['first'], 'cursor-1', 3)
  requestDiscussionReferenceResource.write(identity, first)
  await loadMoreRequestDiscussionReferences(identity, async () => {
    throw new Error('offline')
  })
  const snapshot = requestDiscussionReferenceResource.getSnapshot(identity)
  assert.equal(snapshot.value, first)
  assert.equal((snapshot.error as Error).message, 'offline')
  assert.equal(snapshot.pending, false)
})
