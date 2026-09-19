import assert from 'node:assert/strict'
import test from 'node:test'
import type {
  LoadRepliesInput,
  RequestDiscussionRepliesPage,
} from './request-discussion-api'
import {
  loadLinkedRequestDiscussionReply,
  loadOlderRequestDiscussionReplies,
  openRequestDiscussionReplies,
  requestDiscussionRepliesResource,
  type RequestDiscussionRepliesReadContext,
} from './request-discussion-replies-resource'
import {
  acknowledgeReply,
  insertOptimisticReply,
  markReplyFailed,
} from './request-discussion-replies-model'
import { reply } from './request-discussion-test-fixtures'

test('same linked targets join pending work after reopening and distinct targets serialize', async () => {
  requestDiscussionRepliesResource.clear()
  const first = deferred<RequestDiscussionRepliesPage>()
  const second = deferred<RequestDiscussionRepliesPage>()
  const calls: LoadRepliesInput[] = []
  const pending = [first, second]
  const context = readContext((input) => {
    calls.push(input)
    return pending.shift()!.promise
  })
  const session = openRequestDiscussionReplies('viewer/access/request/thread')

  const firstLoad = loadLinkedRequestDiscussionReply(
    session,
    context,
    'target-one',
  )
  const reopened = openRequestDiscussionReplies(
    'viewer/access/request/thread',
  )
  const joinedLoad = loadLinkedRequestDiscussionReply(
    reopened,
    context,
    'target-one',
  )
  const serializedLoad = loadLinkedRequestDiscussionReply(
    reopened,
    context,
    'target-two',
  )

  assert.equal(joinedLoad, firstLoad)
  assert.equal(reopened.target?.promise, firstLoad)
  assert.deepEqual(calls.map(({ reply: target }) => target), ['target-one'])

  first.resolve(replyPage([reply('target-one', 1)], null))
  assert.equal(await firstLoad, true)
  assert.equal(await joinedLoad, true)
  await Promise.resolve()
  assert.deepEqual(calls.map(({ reply: target }) => target), [
    'target-one',
    'target-two',
  ])

  second.resolve(replyPage([reply('target-two', 2)], null))
  assert.equal(await serializedLoad, true)
  assert.deepEqual(
    openRequestDiscussionReplies(
      'viewer/access/request/thread',
    ).replies.map(({ id }) => id),
    ['target-one', 'target-two'],
  )
})

test('linked target loading preserves the older-page cursor', async () => {
  requestDiscussionRepliesResource.clear()
  const newestPage = deferred<RequestDiscussionRepliesPage>()
  const targetPage = deferred<RequestDiscussionRepliesPage>()
  const pending = [newestPage, targetPage]
  const calls: LoadRepliesInput[] = []
  const context = readContext((input) => {
    calls.push(input)
    return pending.shift()!.promise
  })
  const session = openRequestDiscussionReplies('request')

  const newestLoad = loadOlderRequestDiscussionReplies(
    session,
    context,
    true,
  )!
  assert.equal(
    loadOlderRequestDiscussionReplies(session, context, true),
    undefined,
  )
  assert.equal(calls.length, 1)
  newestPage.resolve(replyPage([reply('new', 10)], 10))
  await newestLoad

  const targetLoad = loadLinkedRequestDiscussionReply(
    session,
    context,
    'target',
  )
  targetPage.resolve(replyPage([reply('target', 2)], 2))
  assert.equal(await targetLoad, true)

  assert.deepEqual(calls, [
    {
      before: undefined,
      discussion_id: 'one',
      owner: 'scope',
      repo: 'scope',
      request_id: 'request-1',
    },
    {
      discussion_id: 'one',
      owner: 'scope',
      repo: 'scope',
      reply: 'target',
      request_id: 'request-1',
    },
  ])
  assert.equal(
    openRequestDiscussionReplies('request').page.nextBeforePosition,
    10,
  )
})

test('a newer preview reloads the newest page before advancing its older cursor', async () => {
  requestDiscussionRepliesResource.clear()
  const initial = deferred<RequestDiscussionRepliesPage>()
  const refreshed = deferred<RequestDiscussionRepliesPage>()
  const older = deferred<RequestDiscussionRepliesPage>()
  const pending = [initial, refreshed, older]
  const calls: LoadRepliesInput[] = []
  const loadReplies = (input: LoadRepliesInput) => {
    calls.push(input)
    return pending.shift()!.promise
  }
  const session = openRequestDiscussionReplies('request')

  const initialLoad = loadOlderRequestDiscussionReplies(
    session,
    readContext(loadReplies, [reply('one-hundred', 100)]),
    true,
  )!
  initial.resolve(
    replyPage(
      [reply('fifty-one', 51), reply('one-hundred', 100)],
      51,
    ),
  )
  await initialLoad

  const advancedContext = readContext(loadReplies, [
    reply('one-hundred-four', 104),
  ])
  const refreshLoad = loadOlderRequestDiscussionReplies(
    session,
    advancedContext,
    true,
  )!
  refreshed.resolve(
    replyPage(
      [reply('fifty-five', 55), reply('one-hundred-four', 104)],
      55,
    ),
  )
  await refreshLoad

  const olderLoad = loadOlderRequestDiscussionReplies(
    session,
    advancedContext,
    true,
  )!
  older.resolve(replyPage([reply('one', 1)], null))
  await olderLoad

  assert.deepEqual(calls.map(({ before }) => before), [
    undefined,
    undefined,
    55,
  ])
})

test('failed page loads retain replies and can be retried through the session owner', async () => {
  requestDiscussionRepliesResource.clear()
  const initial = deferred<RequestDiscussionRepliesPage>()
  const failed = deferred<RequestDiscussionRepliesPage>()
  const retried = deferred<RequestDiscussionRepliesPage>()
  const pending = [initial, failed, retried]
  const context = readContext(() => pending.shift()!.promise)
  const session = openRequestDiscussionReplies('request')

  const initialLoad = loadOlderRequestDiscussionReplies(
    session,
    context,
    true,
  )!
  initial.resolve(replyPage([reply('visible', 10)], 10))
  await initialLoad

  const failedLoad = loadOlderRequestDiscussionReplies(
    session,
    context,
    true,
  )!
  failed.reject({})
  await failedLoad
  const failedState = openRequestDiscussionReplies('request')
  assert.deepEqual(failedState.replies.map(({ id }) => id), ['visible'])
  assert.equal(failedState.page.loading, false)
  assert.equal(
    failedState.page.error,
    'Earlier replies could not be loaded.',
  )

  const retry = loadOlderRequestDiscussionReplies(
    session,
    context,
    true,
  )!
  assert.equal(openRequestDiscussionReplies('request').page.error, null)
  retried.resolve(replyPage([reply('older', 2)], null))
  await retry
  const recovered = openRequestDiscussionReplies('request')
  assert.deepEqual(recovered.replies.map(({ id }) => id), [
    'older',
    'visible',
  ])
  assert.equal(recovered.page.error, null)
})

test('failed linked targets settle, clear their operation, and retry', async () => {
  requestDiscussionRepliesResource.clear()
  const failed = deferred<RequestDiscussionRepliesPage>()
  const retried = deferred<RequestDiscussionRepliesPage>()
  const pending = [failed, retried]
  const context = readContext(() => pending.shift()!.promise)
  const session = openRequestDiscussionReplies('request')

  const firstLoad = loadLinkedRequestDiscussionReply(
    session,
    context,
    'target',
  )
  failed.reject({})
  assert.equal(await firstLoad, false)
  await Promise.resolve()
  const failedState = openRequestDiscussionReplies('request')
  assert.equal(failedState.target, null)
  assert.equal(failedState.page.loading, false)
  assert.equal(
    failedState.page.error,
    'Linked reply could not be loaded.',
  )

  const retry = loadLinkedRequestDiscussionReply(
    session,
    context,
    'target',
  )
  assert.equal(openRequestDiscussionReplies('request').page.error, null)
  retried.resolve(replyPage([reply('target', 1)], null))
  assert.equal(await retry, true)
  assert.equal(openRequestDiscussionReplies('request').page.error, null)
})

test('clearing and replacing a session rejects late target and page writes', async () => {
  requestDiscussionRepliesResource.clear()
  const pendingTarget = deferred<RequestDiscussionRepliesPage>()
  const targetContext = readContext(() => pendingTarget.promise)
  const old = openRequestDiscussionReplies('request')
  const targetLoad = loadLinkedRequestDiscussionReply(
    old,
    targetContext,
    'late-target',
  )

  requestDiscussionRepliesResource.clear()
  openRequestDiscussionReplies('request')
  pendingTarget.resolve(replyPage([reply('late-target', 1)], null))
  assert.equal(await targetLoad, true)
  assert.deepEqual(openRequestDiscussionReplies('request').replies, [])

  const pendingPage = deferred<RequestDiscussionRepliesPage>()
  const pageContext = readContext(() => pendingPage.promise)
  const replaced = openRequestDiscussionReplies('page-request')
  const pageLoad = loadOlderRequestDiscussionReplies(
    replaced,
    pageContext,
    true,
  )!

  requestDiscussionRepliesResource.clear()
  const replacement = openRequestDiscussionReplies('page-request')
  pendingPage.resolve(replyPage([reply('late-page', 2)], null))
  await pageLoad
  assert.equal(replacement.read(), replacement)
  assert.deepEqual(openRequestDiscussionReplies('page-request').replies, [])
  assert.equal(
    openRequestDiscussionReplies('page-request').page.loading,
    false,
  )
})

test('viewer and access scopes retain separate reply sessions', async () => {
  requestDiscussionRepliesResource.clear()
  const viewerSession = openRequestDiscussionReplies(
    'viewer/access/request/thread',
  )
  const context = readContext(async () =>
    replyPage([reply('viewer-reply', 1)], null),
  )

  await loadOlderRequestDiscussionReplies(viewerSession, context, true)

  assert.deepEqual(viewerSession.read()?.replies.map(({ id }) => id), [
    'viewer-reply',
  ])
  assert.deepEqual(
    openRequestDiscussionReplies(
      'other-viewer/access/request/thread',
    ).replies,
    [],
  )
  assert.deepEqual(
    openRequestDiscussionReplies(
      'viewer/other-access/request/thread',
    ).replies,
    [],
  )
})

test('optimistic failure, retry and acknowledgment update the persistent reply owner', () => {
  requestDiscussionRepliesResource.clear()
  const session = openRequestDiscussionReplies('request')
  session.update((current) =>
    insertOptimisticReply(
      current,
      reply('client', Number.MAX_SAFE_INTEGER, { pending: 'sending' }),
    ),
  )
  session.update((current) => markReplyFailed(current, 'client'))
  const reopened = openRequestDiscussionReplies('request')
  assert.equal(reopened.replies[0]?.pending, 'failed')
  reopened.update((current) =>
    insertOptimisticReply(
      current,
      reply('client', Number.MAX_SAFE_INTEGER, { pending: 'sending' }),
    ),
  )
  session.update((current) =>
    acknowledgeReply(current, 'client', reply('server', 4)),
  )
  assert.deepEqual(
    openRequestDiscussionReplies('request').replies.map(({ id, pending }) => ({
      id,
      pending,
    })),
    [{ id: 'server', pending: undefined }],
  )
})

function readContext(
  loadReplies: RequestDiscussionRepliesReadContext['loadReplies'],
  latestReplies: RequestDiscussionRepliesReadContext['latestReplies'] = [],
): RequestDiscussionRepliesReadContext {
  return {
    discussionId: 'one',
    latestReplies,
    loadReplies,
    params: {
      owner: 'scope',
      repo: 'scope',
      request_id: 'request-1',
    },
  }
}

function replyPage(
  replies: RequestDiscussionRepliesPage['replies'],
  nextBeforePosition: number | null,
): RequestDiscussionRepliesPage {
  return {
    next_before_position: nextBeforePosition,
    replies,
  }
}

function deferred<T>() {
  let reject!: (reason?: unknown) => void
  let resolve!: (value: T) => void
  const promise = new Promise<T>((complete, fail) => {
    reject = fail
    resolve = complete
  })
  return { promise, reject, resolve }
}
