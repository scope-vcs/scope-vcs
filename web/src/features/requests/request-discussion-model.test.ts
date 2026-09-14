import assert from 'node:assert/strict'
import test from 'node:test'
import { discussion, discussionPage } from './request-discussion-test-fixtures'
import {
  appendDiscussionPage,
  applyDiscussionChanges,
  collectionFromPage,
  discussionRepliesAreCollapsed,
  includeFocusedDiscussion,
  insertOptimisticDiscussion,
  markDiscussionRead,
  mergeDiscussion,
  mergeRefreshedDiscussionPage,
  reconcileDiscussionMutation,
} from './request-discussion-model'
import type {
  RequestDiscussionView,
} from './request-discussion-types'

test('includes an explicitly focused discussion beyond the first page', () => {
  const result = includeFocusedDiscussion(
    discussionPage([discussion('newest', 10)], 10, 'older'),
    discussionPage([discussion('focused', 2)], 10),
  )
  assert.deepEqual(result?.discussions.map(({ id }) => id), ['newest', 'focused'])
  assert.equal(result?.next_cursor, 'older')
})

test('open discussion replies expand by default and explicit thread state wins', () => {
  const open = discussion('open', 1)
  const resolved = {
    ...discussion('resolved', 2),
    initiallyResolved: true,
    status: 'Resolved' as const,
  }

  assert.equal(discussionRepliesAreCollapsed(open), false)
  assert.equal(discussionRepliesAreCollapsed(resolved), true)
  assert.equal(discussionRepliesAreCollapsed({ ...open, expanded: false }), true)
  assert.equal(discussionRepliesAreCollapsed({ ...resolved, expanded: true }), false)
})

test('appends cursor pages without duplicating discussions', () => {
  const first = collectionFromPage(discussionPage([discussion('one', 5), discussion('two', 4)], 5, 'next'))
  const result = appendDiscussionPage(first, discussionPage([discussion('two', 4), discussion('three', 3)], 5))
  assert.deepEqual(result.order, ['three', 'two', 'one'])
  assert.equal(result.nextCursor, null)
})

test('pagination cannot replace a newer discussion projection', () => {
  const base = collectionFromPage(discussionPage([discussion('one', 12)], 12, 'older'))
  const current = mergeDiscussion(base, {
    ...discussion('one', 12),
    expanded: true,
  })
  const paged = appendDiscussionPage(current, discussionPage([discussion('one', 7), discussion('older', 6)], 7))

  assert.equal(paged.byId.get('one')?.last_activity_position, 12)
  assert.equal(paged.byId.get('one')?.expanded, true)
  assert.equal(paged.snapshotVersion, 12)
  assert.equal(paged.nextCursor, null)
  assert.deepEqual(paged.order, ['older', 'one'])
})

test('stale refresh merges safe rows without changing order or cursor', () => {
  const current = collectionFromPage(discussionPage([discussion('one', 11), discussion('two', 10)], 11, 'current-cursor'))
  const refreshed = mergeRefreshedDiscussionPage(
    { ...current, order: ['one', 'two'] },
    discussionPage([discussion('one', 9), discussion('older', 8)], 9, 'stale-cursor'),
    false,
  )

  assert.equal(refreshed.byId.get('one')?.last_activity_position, 11)
  assert.equal(refreshed.snapshotVersion, 11)
  assert.equal(refreshed.nextCursor, 'current-cursor')
  assert.deepEqual(refreshed.order, ['older', 'one', 'two'])
})

test('non-authoritative refresh does not advance past unseen changes', () => {
  const current = collectionFromPage(discussionPage([discussion('one', 10)], 10))
  const refreshed = mergeRefreshedDiscussionPage(
    current,
    discussionPage([discussion('new-root', 12)], 12),
    false,
  )

  assert.equal(refreshed.byId.get('new-root')?.last_activity_position, 12)
  assert.equal(refreshed.snapshotVersion, 10)
})

test('authoritative refresh restores page order and preserves active UI rows', () => {
  const initial = collectionFromPage(discussionPage([discussion('one', 5), discussion('two', 4)], 5, 'old-cursor'))
  const expanded = mergeDiscussion(initial, {
    ...discussion('one', 5),
    expanded: true,
  })
  const pending = insertOptimisticDiscussion(expanded, {
    ...discussion('client-1', Number.MAX_SAFE_INTEGER),
    pending: 'failed',
  })
  const refreshed = mergeRefreshedDiscussionPage(
    { ...pending, order: ['one', 'two', 'client-1'] },
    discussionPage([discussion('one', 6), discussion('two', 4)], 6, 'fresh-cursor'),
    true,
  )

  assert.deepEqual(refreshed.order, ['two', 'one', 'client-1'])
  assert.equal(refreshed.byId.get('one')?.expanded, true)
  assert.equal(refreshed.byId.get('client-1')?.pending, 'failed')
  assert.equal(refreshed.nextCursor, 'fresh-cursor')
  assert.equal(refreshed.snapshotVersion, 6)
})

test('authoritative refresh replaces an accepted optimistic discussion', () => {
  const initial = collectionFromPage(discussionPage([], 5))
  const pending = insertOptimisticDiscussion(initial, {
    ...discussion('client-1', Number.MAX_SAFE_INTEGER),
    pending: 'failed',
  })
  const accepted = {
    ...discussion('server-1', 6),
    client_discussion_id: 'client-1',
  }

  const refreshed = mergeRefreshedDiscussionPage(
    pending,
    discussionPage([accepted], 6),
    true,
  )

  assert.equal(refreshed.byId.has('client-1'), false)
  assert.equal(refreshed.byId.get('server-1')?.pending, undefined)
  assert.deepEqual(refreshed.order, ['server-1'])
})

test('authoritative refresh drops expanded rows outside the refreshed page', () => {
  const initial = collectionFromPage(discussionPage([discussion('visible', 6), discussion('older', 5)], 6))
  const expanded = mergeDiscussion(initial, {
    ...discussion('older', 5),
    expanded: true,
  })
  const refreshed = mergeRefreshedDiscussionPage(
    expanded,
    discussionPage([discussion('visible', 7)], 7, 'older-page'),
    true,
  )

  assert.equal(refreshed.byId.has('older'), false)
  assert.deepEqual(refreshed.order, ['visible'])
  assert.equal(refreshed.snapshotVersion, 7)
})

test('changes never move an entity or the collection snapshot backward', () => {
  const base = collectionFromPage(discussionPage([discussion('one', 12)], 12))
  const current = mergeDiscussion(base, {
    ...discussion('one', 12),
    expanded: true,
  })
  const changed = applyDiscussionChanges(current, [discussion('one', 7)], 10)

  assert.equal(changed.byId.get('one')?.last_activity_position, 12)
  assert.equal(changed.byId.get('one')?.expanded, true)
  assert.equal(changed.snapshotVersion, 12)
})

test('realtime patches a discussion without moving it under the cursor', () => {
  const collection = collectionFromPage(discussionPage([discussion('one', 5), discussion('two', 4)], 5))
  const patched = applyDiscussionChanges(
    collection,
    [discussion('two', 9)],
    9,
  )
  assert.deepEqual(patched.order, ['two', 'one'])
})

test('realtime appends a new root without reordering visible roots', () => {
  const collection = collectionFromPage(discussionPage([discussion('one', 5), discussion('two', 4)], 5))
  const patched = applyDiscussionChanges(
    collection,
    [discussion('three', 8)],
    8,
  )
  assert.deepEqual(patched.order, ['two', 'one', 'three'])
  assert.equal(patched.snapshotVersion, 8)
})

test('realtime inserts an unseen older root by its opened position', () => {
  const collection = collectionFromPage(discussionPage([discussion('one', 5), discussion('two', 4)], 5, 'older'))
  const older = {
    ...discussion('older', 9),
    opened_position: 3,
  }
  const patched = applyDiscussionChanges(
    collection,
    [older],
    9,
  )
  assert.deepEqual(patched.order, ['older', 'two', 'one'])
})

test('timeline keeps newly discovered resolved roots', () => {
  const collection = collectionFromPage(discussionPage([discussion('one', 5)], 5))
  const resolved = {
    ...discussion('resolved', 8),
    resolved_at_unix: 8,
    status: 'Resolved' as const,
  }
  const patched = applyDiscussionChanges(
    collection,
    [resolved],
    8,
  )
  assert.deepEqual(patched.order, ['one', 'resolved'])
  assert.equal(patched.snapshotVersion, 8)
})

test('timeline keeps roots in place when they become resolved', () => {
  const collection = collectionFromPage(discussionPage([discussion('one', 5), discussion('two', 4)], 5))
  const resolved = {
    ...discussion('one', 8),
    resolved_at_unix: 8,
    status: 'Resolved' as const,
  }
  const patched = applyDiscussionChanges(
    collection,
    [resolved],
    8,
  )
  assert.deepEqual(patched.order, ['two', 'one'])
  assert.equal(patched.byId.get('one')?.status, 'Resolved')
  assert.equal(patched.snapshotVersion, 8)
})

test('optimistic roots are replaced in their visible position', () => {
  const collection = collectionFromPage(discussionPage([discussion('one', 5)], 5))
  const optimistic = {
    ...discussion('client-1', 6),
    pending: 'sending',
  } satisfies RequestDiscussionView
  const pending = insertOptimisticDiscussion(collection, optimistic)
  assert.deepEqual(pending.order, ['one', 'client-1'])
})

test('optimistic acknowledgement is ordered around concurrent roots', () => {
  const collection = collectionFromPage(discussionPage([discussion('one', 5)], 5))
  const optimistic = {
    ...discussion('client-1', Number.MAX_SAFE_INTEGER),
    pending: 'sending' as const,
  }
  const pending = insertOptimisticDiscussion(collection, optimistic)
  const withConcurrentRoot = mergeDiscussion(
    pending,
    discussion('later', 7),
  )

  const acknowledged = reconcileDiscussionMutation(
    withConcurrentRoot,
    discussion('acknowledged', 6),
    'client-1',
  )

  assert.deepEqual(acknowledged.order, ['one', 'acknowledged', 'later'])
})

test('same-id optimistic acknowledgement clears pending state', () => {
  const collection = collectionFromPage(discussionPage([], 5))
  const pending = insertOptimisticDiscussion(collection, {
    ...discussion('client-1', Number.MAX_SAFE_INTEGER),
    pending: 'sending',
  })
  const acknowledged = reconcileDiscussionMutation(
    pending,
    discussion('client-1', 6),
  )

  assert.equal(acknowledged.byId.get('client-1')?.pending, undefined)
  assert.equal(acknowledged.byId.get('client-1')?.last_activity_position, 6)
})

test('catch-up can acknowledge a same-id optimistic discussion', () => {
  const collection = collectionFromPage(discussionPage([], 5))
  const pending = insertOptimisticDiscussion(collection, {
    ...discussion('client-1', Number.MAX_SAFE_INTEGER),
    pending: 'sending',
  })
  const acknowledged = applyDiscussionChanges(
    pending,
    [discussion('client-1', 6)],
    6,
  )

  assert.equal(acknowledged.byId.get('client-1')?.pending, undefined)
  assert.equal(acknowledged.byId.get('client-1')?.last_activity_position, 6)
  assert.equal(acknowledged.snapshotVersion, 6)
})

test('catch-up replaces an optimistic discussion by its client id', () => {
  const collection = collectionFromPage(discussionPage([], 5))
  const pending = insertOptimisticDiscussion(collection, {
    ...discussion('client-1', Number.MAX_SAFE_INTEGER),
    pending: 'sending',
  })
  const accepted = applyDiscussionChanges(
    pending,
    [{ ...discussion('server-1', 6), client_discussion_id: 'client-1' }],
    6,
  )

  assert.equal(accepted.byId.has('client-1'), false)
  assert.equal(accepted.byId.get('server-1')?.pending, undefined)
  assert.deepEqual(accepted.order, ['server-1'])
  assert.equal(accepted.snapshotVersion, 6)
})

test('client id collisions from another author preserve the optimistic row', () => {
  const collection = collectionFromPage(discussionPage([], 5))
  const pending = insertOptimisticDiscussion(collection, {
    ...discussion('client-1', Number.MAX_SAFE_INTEGER),
    pending: 'sending',
  })
  const otherAuthor = {
    ...discussion('server-1', 6),
    author: { handle: 'river', id: 'user-river' },
    client_discussion_id: 'client-1',
  }
  const changed = applyDiscussionChanges(pending, [otherAuthor], 6)

  assert.equal(changed.byId.get('client-1')?.pending, 'sending')
  assert.equal(changed.byId.get('server-1')?.author.id, 'user-river')
  assert.deepEqual(changed.order, ['server-1', 'client-1'])
})

test('mutation responses do not advance the authoritative catch-up cursor', () => {
  const collection = collectionFromPage(discussionPage([discussion('one', 5)], 5))
  const patched = mergeDiscussion(collection, discussion('one', 9))
  assert.equal(patched.snapshotVersion, 5)
  assert.equal(patched.byId.get('one')?.last_activity_position, 9)
})

test('mark read is monotonic in the client projection', () => {
  const collection = collectionFromPage(discussionPage([{ ...discussion('one', 5), unread_count: 3 }], 5))
  const read = markDiscussionRead(collection, 'one')
  assert.equal(read.byId.get('one')?.unread_count, 0)
  assert.equal(markDiscussionRead(read, 'one'), read)
})
