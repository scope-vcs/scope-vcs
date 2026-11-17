import assert from 'node:assert/strict'
import test, { mock } from 'node:test'
import {
  readRequestDiscussionScroll,
  requestDiscussionCacheKey,
  requestDiscussionResource,
  requestDiscussionState,
  syncRequestDiscussionProjections,
  updateRequestDiscussion,
  updateRequestDiscussionCollection,
  writeRequestDiscussionScroll,
} from './request-discussion-cache'
import { resetViewerState } from '../../lib/viewer-state'
import { mergeDiscussion } from './request-discussion-model'
import { discussion } from './request-discussion-test-fixtures'

const page = { discussions: [discussion('one', 1)], next_cursor: 'older', snapshot_version: 1 }

function retain(key: string, loaded: Parameters<typeof requestDiscussionState>[0]) {
  const state = requestDiscussionState(loaded)
  requestDiscussionResource.write(key, state)
  return state
}

test('keys timeline views by viewer, repository access scope, and request', () => {
  const base = { repoId: 'scope/demo/member', requestId: 'request-1', viewerId: 'maya' }
  const key = requestDiscussionCacheKey(base)
  assert.equal(key, requestDiscussionCacheKey({ ...base }))
  for (const change of [{ requestId: 'request-2' }, { viewerId: 'ravi' }, { repoId: 'scope/demo/public' }]) {
    assert.notEqual(key, requestDiscussionCacheKey({ ...base, ...change }))
  }
})

test('reopening finds the subscribed collection, expansion, scroll and pending pagination', () => {
  resetViewerState()
  const { epoch } = retain('request', page)
  let notifications = 0
  const unsubscribe = requestDiscussionResource.subscribe('request', () => notifications++)
  updateRequestDiscussionCollection('request', epoch, (current) => mergeDiscussion(current, { ...discussion('one', 1), expanded: true }), false)
  updateRequestDiscussion('request', epoch, { loadingMore: true })
  writeRequestDiscussionScroll('request', 240)
  unsubscribe()

  const reopened = requestDiscussionResource.peek('request')
  assert.ok(reopened)
  assert.equal(reopened.collection.byId.get('one')?.expanded, true)
  assert.equal(reopened.dataGeneration, 0)
  assert.equal(reopened.loadingMore, true)
  assert.equal(readRequestDiscussionScroll('request'), 240)
  assert.equal(notifications, 3)
})

test('bounds retained views and ignores late writes after eviction or reset', () => {
  resetViewerState()
  const original = retain('request', page)
  for (let index = 0; index < 8; index++) retain(`other-${index}`, page)
  assert.equal(requestDiscussionResource.peek('request'), null)
  updateRequestDiscussion('request', original.epoch, { error: 'late error' })
  assert.equal(requestDiscussionResource.peek('request'), null)
  const replacement = retain('request', page)
  updateRequestDiscussion('request', original.epoch, { loadingMore: true })
  assert.equal(requestDiscussionResource.peek('request')?.loadingMore, false)
  resetViewerState()
  updateRequestDiscussion('request', replacement.epoch, { error: 'late error' })
  assert.equal(requestDiscussionResource.peek('request'), null)
})

const lineAnchor = {
  revision_id: 'revision-1', revision_position: 1, commit_oid: null, path: '/src/lib.rs',
  lines: { side: 'new', start_line: 3, end_line: 4 }, latest: null,
} as const

function mappedTo(revisionId: string) {
  return { ...lineAnchor, latest: { kind: 'mapped', revision_id: revisionId, start_line: 5, end_line: 6 } } as const
}

function linedPage(ids: string[], revisionId: string | null, nextCursor: string | null = null) {
  return {
    discussions: ids.map((id, index) => discussion(id, 1000 - index, { anchor: revisionId ? mappedTo(revisionId) : lineAnchor })),
    next_cursor: nextCursor,
    snapshot_version: 1000,
  }
}

function latestOf(id: string) {
  return requestDiscussionResource.peek('request')?.collection.byId.get(id)?.anchor?.latest ?? null
}

test('a changed request head refreshes every retained line discussion, across pages', async () => {
  resetViewerState()
  const ids = Array.from({ length: 101 }, (_, index) => `line-${index}`)
  const { epoch } = retain('request', { ...linedPage(ids, null), discussions: [...linedPage(ids, null).discussions, discussion('plain', 1)] })
  const load = mock.fn(async ({ cursor }: { cursor?: string; limit: number }) => cursor
    ? linedPage(ids.slice(100), 'revision-2')
    : linedPage(ids.slice(0, 100), 'revision-2', 'older'))

  await syncRequestDiscussionProjections('request', epoch, 'head-1', load)
  assert.equal(load.mock.callCount(), 0)
  await syncRequestDiscussionProjections('request', epoch, 'head-2', load)

  assert.equal(load.mock.callCount(), 2)
  assert.deepEqual(load.mock.calls.map((call) => call.arguments[0]), [{ cursor: undefined, limit: 100 }, { cursor: 'older', limit: 100 }])
  assert.equal(latestOf('line-0')?.revision_id, 'revision-2')
  assert.equal(latestOf('line-100')?.revision_id, 'revision-2')
  assert.ok(requestDiscussionResource.peek('request')?.collection.byId.get('plain'))
  await syncRequestDiscussionProjections('request', epoch, 'head-2', load)
  assert.equal(load.mock.callCount(), 2)
})

test('reopening a retained view after a missed push refreshes its projections', async () => {
  resetViewerState()
  const { epoch } = retain('request', linedPage(['lined'], null))
  await syncRequestDiscussionProjections('request', epoch, 'head-1', async () => linedPage(['lined'], null))
  const load = mock.fn(async () => linedPage(['lined'], 'revision-2'))

  await syncRequestDiscussionProjections('request', epoch, 'head-2', load)

  assert.equal(load.mock.callCount(), 1)
  assert.equal(latestOf('lined')?.revision_id, 'revision-2')
})

test('a projection response for an older head never replaces a newer one', async () => {
  resetViewerState()
  const { epoch } = retain('request', linedPage(['lined'], null))
  await syncRequestDiscussionProjections('request', epoch, 'head-1', async () => linedPage(['lined'], null))
  let resolveOlder = (_page: ReturnType<typeof linedPage>) => {}
  const older = syncRequestDiscussionProjections('request', epoch, 'head-2', () => new Promise((resolve) => { resolveOlder = resolve }))
  const newer = syncRequestDiscussionProjections('request', epoch, 'head-3', async () => linedPage(['lined'], 'revision-3'))
  await newer
  resolveOlder(linedPage(['lined'], 'revision-2'))
  await older

  assert.equal(latestOf('lined')?.revision_id, 'revision-3')
})
