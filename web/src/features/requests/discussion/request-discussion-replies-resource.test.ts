import assert from 'node:assert/strict'
import test from 'node:test'
import type { RequestDiscussionRepliesPage } from './request-discussion-api'
import {
  createRequestDiscussionReplyReads,
  openRequestDiscussionReplies,
  requestDiscussionRepliesResource,
} from './request-discussion-replies-resource'
import {
  acknowledgeReply,
  insertOptimisticReply,
  markReplyFailed,
} from './request-discussion-replies-model'
import { reply } from './request-discussion-test-fixtures'

type LatestReplies = Parameters<typeof createRequestDiscussionReplyReads>[2]
type LoadReplies = Parameters<typeof createRequestDiscussionReplyReads>[1]
type LoadRepliesInput = Parameters<LoadReplies>[0]

test.beforeEach(() => requestDiscussionRepliesResource.clear())

test('same linked targets join pending work after reopening and distinct targets serialize', async () => {
  const fixture = replyReads('viewer/access/request/thread')

  const firstLoad = fixture.loadReplyTarget('target-one')
  const reopened = openRequestDiscussionReplies(fixture.key)
  const reopenedReads = fixture.reads(reopened)
  const joinedLoad = reopenedReads.loadReplyTarget('target-one')
  const serializedLoad = reopenedReads.loadReplyTarget('target-two')

  assert.equal(joinedLoad, firstLoad)
  assert.equal(reopened.target?.promise, firstLoad)
  assert.deepEqual(fixture.calls, [{ reply: 'target-one' }])

  fixture.requests[0]!.resolve(page([reply('target-one', 1)], null))
  assert.equal(await firstLoad, true)
  assert.equal(await joinedLoad, true)
  await Promise.resolve()
  assert.deepEqual(fixture.calls, [
    { reply: 'target-one' },
    { reply: 'target-two' },
  ])

  fixture.requests[1]!.resolve(page([reply('target-two', 2)], null))
  assert.equal(await serializedLoad, true)
  assert.deepEqual(currentIds(fixture.key), ['target-one', 'target-two'])
})

test('linked target loading preserves the older-page cursor', async () => {
  const fixture = replyReads()

  const newestLoad = fixture.loadOlderReplies()!
  assert.equal(fixture.loadOlderReplies(), undefined)
  assert.deepEqual(fixture.calls, [{ before: undefined }])
  fixture.requests[0]!.resolve(page([reply('new', 10)], 10))
  await newestLoad

  const targetLoad = fixture.loadReplyTarget('target')
  fixture.requests[1]!.resolve(page([reply('target', 2)], 2))
  assert.equal(await targetLoad, true)

  assert.deepEqual(fixture.calls, [
    { before: undefined },
    { reply: 'target' },
  ])
  assert.equal(openRequestDiscussionReplies(fixture.key).page.nextBeforePosition, 10)
})

test('a newer preview reloads the newest page before advancing its older cursor', async () => {
  const fixture = replyReads()
  const initial = fixture.reads(undefined, [reply('one-hundred', 100)])

  const initialLoad = initial.loadOlderReplies()!
  fixture.requests[0]!.resolve(
    page([reply('fifty-one', 51), reply('one-hundred', 100)], 51),
  )
  await initialLoad

  const advanced = fixture.reads(undefined, [reply('one-hundred-four', 104)])
  const refreshLoad = advanced.loadOlderReplies()!
  fixture.requests[1]!.resolve(
    page([reply('fifty-five', 55), reply('one-hundred-four', 104)], 55),
  )
  await refreshLoad

  const olderLoad = advanced.loadOlderReplies()!
  fixture.requests[2]!.resolve(page([reply('one', 1)], null))
  await olderLoad

  assert.deepEqual(fixture.calls.map(({ before }) => before), [
    undefined,
    undefined,
    55,
  ])
})

test('failed page loads retain replies and can be retried through the session owner', async () => {
  const fixture = replyReads()

  const initialLoad = fixture.loadOlderReplies()!
  fixture.requests[0]!.resolve(page([reply('visible', 10)], 10))
  await initialLoad

  const failedLoad = fixture.loadOlderReplies()!
  fixture.requests[1]!.reject({})
  await failedLoad
  const failed = openRequestDiscussionReplies(fixture.key)
  assert.deepEqual(failed.replies.map(({ id }) => id), ['visible'])
  assert.equal(failed.page.loading, false)
  assert.equal(failed.page.error, 'Earlier replies could not be loaded.')

  const retry = fixture.loadOlderReplies()!
  assert.equal(openRequestDiscussionReplies(fixture.key).page.error, null)
  fixture.requests[2]!.resolve(page([reply('older', 2)], null))
  await retry
  const recovered = openRequestDiscussionReplies(fixture.key)
  assert.deepEqual(recovered.replies.map(({ id }) => id), ['older', 'visible'])
  assert.equal(recovered.page.error, null)
})

test('failed linked targets settle, clear their operation, and retry', async () => {
  const fixture = replyReads()

  const firstLoad = fixture.loadReplyTarget('target')
  fixture.requests[0]!.reject({})
  assert.equal(await firstLoad, false)
  await Promise.resolve()
  const failed = openRequestDiscussionReplies(fixture.key)
  assert.equal(failed.target, null)
  assert.equal(failed.page.loading, false)
  assert.equal(failed.page.error, 'Linked reply could not be loaded.')

  const retry = fixture.loadReplyTarget('target')
  assert.equal(openRequestDiscussionReplies(fixture.key).page.error, null)
  fixture.requests[1]!.resolve(page([reply('target', 1)], null))
  assert.equal(await retry, true)
  assert.equal(openRequestDiscussionReplies(fixture.key).page.error, null)
})

test('clearing and replacing a session rejects late target and page writes', async () => {
  const targetFixture = replyReads()
  const targetLoad = targetFixture.loadReplyTarget('late-target')

  requestDiscussionRepliesResource.clear()
  openRequestDiscussionReplies(targetFixture.key)
  targetFixture.requests[0]!.resolve(page([reply('late-target', 1)], null))
  assert.equal(await targetLoad, true)
  assert.deepEqual(currentIds(targetFixture.key), [])

  const pageFixture = replyReads('page-request')
  const pageLoad = pageFixture.loadOlderReplies()!
  requestDiscussionRepliesResource.clear()
  const replacement = openRequestDiscussionReplies(pageFixture.key)
  pageFixture.requests[0]!.resolve(page([reply('late-page', 2)], null))
  await pageLoad

  assert.equal(replacement.read(), replacement)
  assert.deepEqual(currentIds(pageFixture.key), [])
  assert.equal(openRequestDiscussionReplies(pageFixture.key).page.loading, false)
})

test('viewer and access scopes retain separate reply sessions', async () => {
  const fixture = replyReads('viewer/access/request/thread')
  const load = fixture.loadOlderReplies()!
  fixture.requests[0]!.resolve(page([reply('viewer-reply', 1)], null))
  await load

  assert.deepEqual(fixture.session.read()?.replies.map(({ id }) => id), [
    'viewer-reply',
  ])
  assert.deepEqual(currentIds('other-viewer/access/request/thread'), [])
  assert.deepEqual(currentIds('viewer/other-access/request/thread'), [])
})

test('optimistic failure, retry and acknowledgment update the persistent reply owner', () => {
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
  session.update((current) => acknowledgeReply(current, 'client', reply('server', 4)))
  assert.deepEqual(
    openRequestDiscussionReplies('request').replies.map(({ id, pending }) => ({
      id,
      pending,
    })),
    [{ id: 'server', pending: undefined }],
  )
})

function replyReads(key = 'request') {
  const calls: LoadRepliesInput[] = []
  const requests: ReturnType<typeof deferred<RequestDiscussionRepliesPage>>[] = []
  const session = openRequestDiscussionReplies(key)
  const loadReplies: LoadReplies = (input) => {
    calls.push(input)
    const request = deferred<RequestDiscussionRepliesPage>()
    requests.push(request)
    return request.promise
  }
  const reads = (
    owner = session,
    latestReplies: LatestReplies = [],
    hasOlderReplies = true,
  ) =>
    createRequestDiscussionReplyReads(
      owner,
      loadReplies,
      latestReplies,
      hasOlderReplies,
    )

  return { calls, key, requests, session, reads, ...reads() }
}

function currentIds(key: string) {
  return openRequestDiscussionReplies(key).replies.map(({ id }) => id)
}

function page(
  replies: RequestDiscussionRepliesPage['replies'],
  nextBeforePosition: number | null,
): RequestDiscussionRepliesPage {
  return { next_before_position: nextBeforePosition, replies }
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
