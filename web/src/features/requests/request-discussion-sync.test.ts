import assert from 'node:assert/strict'
import test from 'node:test'
import { deferred, discussion as discussionFixture } from './request-discussion-test-fixtures'
import {
  applyDiscussionChanges,
  collectionFromPage,
  type DiscussionCollection,
} from './request-discussion-model'
import { createRequestDiscussionSync } from './request-discussion-sync'
import type {
  RequestDiscussion,
  RequestDiscussionChanges,
  RequestDiscussionPage,
} from './request-discussion-types'

test('catch-up coalesces a newer target while a request is in flight', async () => {
  let collection = pageCollection('request-a', 5)
  const first = deferred<RequestDiscussionChanges>()
  const afters: number[] = []
  const sync = createRequestDiscussionSync({
    onCatchUpError: () => {},
    getCollection: () => collection,
    loadChanges: (after) => {
      afters.push(after)
      return after === 5
        ? first.promise
        : Promise.resolve(
            changeBatch([discussion('three', 12, 'request-a')], 12),
          )
    },
    setCollection: (next) => {
      collection = next
    },
  })
  sync.reset('repo-1/request-a')

  const original = sync.catchUp({ target: 10 })
  const coalesced = sync.catchUp({ target: 12 })
  assert.equal(coalesced, original)
  first.resolve(changeBatch([discussion('two', 10, 'request-a')], 10))
  await original

  assert.deepEqual(afters, [5, 10])
  assert.equal(collection.snapshotVersion, 12)
  assert.deepEqual(collection.order, ['one', 'two', 'three'])
})

test('catch-up drains every server page when continuation is explicit', async () => {
  let collection = pageCollection('request-a', 5)
  const firstPage = Array.from({ length: 100 }, (_, index) =>
    discussion(`discussion-${index + 6}`, index + 6, 'request-a'),
  )
  const responses = [
    changeBatch(firstPage, 105, true),
    changeBatch([discussion('discussion-106', 106, 'request-a')], 106),
  ]
  const afters: number[] = []
  const sync = createRequestDiscussionSync({
    onCatchUpError: () => {},
    getCollection: () => collection,
    loadChanges: async (after) => {
      afters.push(after)
      const response = responses.shift()
      assert.ok(response)
      return response
    },
    setCollection: (next) => {
      collection = next
    },
  })
  sync.reset('repo-1/request-a')

  await sync.catchUp()

  assert.deepEqual(afters, [5, 105])
  assert.equal(collection.snapshotVersion, 106)
  assert.equal(collection.order.length, 102)
})

test('lagged catch-up polls until an empty response confirms the head', async () => {
  let collection = pageCollection('request-a', 5)
  const responses = [
    changeBatch([discussion('two', 6, 'request-a')], 6),
    changeBatch([], 7),
  ]
  const afters: number[] = []
  const sync = createRequestDiscussionSync({
    onCatchUpError: () => {},
    getCollection: () => collection,
    loadChanges: async (after) => {
      afters.push(after)
      const response = responses.shift()
      assert.ok(response)
      return response
    },
    setCollection: (next) => {
      collection = next
    },
  })
  sync.reset('repo-1/request-a')

  await sync.catchUp({ lagged: true })

  assert.deepEqual(afters, [5, 6])
  assert.equal(collection.snapshotVersion, 7)
  assert.deepEqual(collection.order, ['one', 'two'])
})

test('a completion from the previous request generation is discarded', async () => {
  let collection = pageCollection('request-a', 5)
  const changes = deferred<RequestDiscussionChanges>()
  const sync = createRequestDiscussionSync({
    onCatchUpError: () => {},
    getCollection: () => collection,
    loadChanges: () => changes.promise,
    setCollection: (next) => {
      collection = next
    },
  })
  sync.reset('repo-1/request-a')
  const oldCatchUp = sync.catchUp({ target: 8 })

  collection = pageCollection('request-b', 20)
  sync.reset('repo-1/request-b')
  changes.resolve(
    changeBatch([discussion('old-request-row', 8, 'request-a')], 8),
  )
  await oldCatchUp

  assert.equal(collection.snapshotVersion, 20)
  assert.deepEqual(collection.order, ['one'])
  assert.equal(collection.byId.has('old-request-row'), false)
})

test('pagination keeps an entity advanced by live catch-up', async () => {
  let collection: DiscussionCollection = {
    ...pageCollection('request-a', 12),
    nextCursor: 'older',
  }
  const olderPage = deferred<ReturnType<typeof page>>()
  const sync = createRequestDiscussionSync({
    onCatchUpError: () => {},
    getCollection: () => collection,
    loadChanges: async () => changeBatch([], collection.snapshotVersion),
    setCollection: (next) => {
      collection = next
    },
  })
  sync.reset('repo-1/request-a')
  const loading = sync.paginate('older', () => olderPage.promise)

  collection = applyDiscussionChanges(
    collection,
    [discussion('one', 15, 'request-a')],
    15,
  )
  olderPage.resolve({
    discussions: [
      discussion('one', 7, 'request-a'),
      discussion('older', 6, 'request-a'),
    ],
    next_cursor: null,
    snapshot_version: 12,
  })

  assert.equal(await loading, true)
  assert.equal(collection.byId.get('one')?.last_activity_position, 15)
  assert.equal(collection.snapshotVersion, 15)
  assert.equal(collection.nextCursor, null)
  assert.deepEqual(collection.order, ['older', 'one'])
})

function pageCollection(requestId: string, position: number) {
  return collectionFromPage(page(requestId, position))
}

function page(requestId: string, position: number): RequestDiscussionPage {
  return {
    discussions: [discussion('one', position, requestId)],
    next_cursor: null,
    snapshot_version: position,
  }
}

function changeBatch(
  discussions: RequestDiscussion[],
  throughPosition: number,
  hasMore = false,
): RequestDiscussionChanges {
  return {
    discussions,
    has_more: hasMore,
    through_position: throughPosition,
  }
}

function discussion(id: string, lastActivity: number, requestId: string) {
  return discussionFixture(id, lastActivity, { request_id: requestId })
}
