import assert from 'node:assert/strict'
import test from 'node:test'
import type {
  RepoChangeEvent,
  RequestAutoMergeResponse,
} from '@/api/types.generated'
import { invalidateRepoResources } from '../repo-detail/repo-resource-invalidation'
import {
  requestAutoMergeIdentity,
  requestAutoMergeResource,
} from './request-auto-merge-resource'

test.beforeEach(() => requestAutoMergeResource.clear())

test('one auto-merge status is reused across mounts in the same access scope', async () => {
  const identity = requestAutoMergeIdentity('viewer-access', 'request')
  let calls = 0
  const load = async () => {
    calls++
    return status()
  }

  await Promise.all([
    requestAutoMergeResource.ensure(identity, '', load),
    requestAutoMergeResource.ensure(identity, '', load),
  ])
  await requestAutoMergeResource.ensure(identity, '', load)

  assert.equal(calls, 1)
  assert.equal(requestAutoMergeResource.peek(identity)?.intent?.status, 'Active')
  assert.equal(
    requestAutoMergeResource.peek(requestAutoMergeIdentity('other-access', 'request')),
    null,
  )
})

test('an action receipt replaces cached status without a follow-up read', async () => {
  const identity = requestAutoMergeIdentity('viewer-access', 'request')
  let calls = 0
  await requestAutoMergeResource.ensure(identity, '', async () => {
    calls++
    return status()
  })

  const generation = requestAutoMergeResource.invalidationGeneration(identity)
  assert.equal(requestAutoMergeResource.writeIfNotInvalidated(identity, generation, {
    ...status(),
    can_cancel: false,
    intent: { ...status().intent!, status: 'Cancelled' },
    waiting_reason: null,
  }), true)
  await requestAutoMergeResource.ensure(identity, '', async () => {
    calls++
    return status()
  })

  assert.equal(calls, 1)
  assert.equal(requestAutoMergeResource.peek(identity)?.intent?.status, 'Cancelled')
})

const invalidatingKinds: RepoChangeEvent['kind'][] = [
  { RequestTimelineChanged: {
    audience: 'Public',
    discussion_id: 'discussion',
    request_id: 'request',
    through_position: 2,
  } },
  { RunChanged: { run_id: 'run', change: 'StatusChanged' as const } },
  { RepositoryChanged: { reason: 'request-auto-merge-updated' } },
  'Lagged' as const,
]

for (const kind of invalidatingKinds) {
  test(`relevant changes refresh retained status: ${JSON.stringify(kind)}`, () => {
    const identity = requestAutoMergeIdentity('viewer-access', 'request')
    const otherScope = requestAutoMergeIdentity('other-access', 'request')
    requestAutoMergeResource.write(identity, status())
    requestAutoMergeResource.write(otherScope, status())

    invalidateRepoResources('viewer-access', {
      incarnation_id: 'incarnation',
      kind,
      repo_id: 'repo',
      version: 2,
    })

    assert.equal(requestAutoMergeResource.getSnapshot(identity).stale, true)
    assert.equal(requestAutoMergeResource.peek(identity)?.intent?.status, 'Active')
    assert.equal(requestAutoMergeResource.getSnapshot(otherScope).stale, false)
  })
}

test('one request timeline change leaves another request reusable', () => {
  const identity = requestAutoMergeIdentity('viewer-access', 'request')
  const other = requestAutoMergeIdentity('viewer-access', 'other-request')
  requestAutoMergeResource.write(identity, status())
  requestAutoMergeResource.write(other, { ...status(), request_id: 'other-request' })

  invalidateRepoResources('viewer-access', {
    incarnation_id: 'incarnation',
    kind: { RequestTimelineChanged: {
      audience: 'Public',
      discussion_id: 'discussion',
      request_id: 'request',
      through_position: 2,
    } },
    repo_id: 'repo',
    version: 2,
  })

  assert.equal(requestAutoMergeResource.getSnapshot(identity).stale, true)
  assert.equal(requestAutoMergeResource.getSnapshot(other).stale, false)
})

test('a run-driven refresh retains active status until it publishes the stop', async () => {
  const identity = requestAutoMergeIdentity('viewer-access', 'request')
  requestAutoMergeResource.write(identity, status())
  invalidateRepoResources('viewer-access', {
    incarnation_id: 'incarnation',
    kind: { RunChanged: { change: 'StatusChanged', run_id: 'run' } },
    repo_id: 'repo',
    version: 2,
  })

  let finish!: (value: RequestAutoMergeResponse) => void
  const next = new Promise<RequestAutoMergeResponse>((resolve) => {
    finish = resolve
  })
  const refresh = requestAutoMergeResource.ensure(identity, '2', () => next)
  assert.equal(requestAutoMergeResource.peek(identity)?.intent?.status, 'Active')
  const stopped = status()
  stopped.can_cancel = false
  stopped.intent = {
    ...stopped.intent!,
    reason: 'ChecksFailed',
    status: 'Stopped',
  }
  stopped.waiting_reason = null
  finish(stopped)
  await refresh

  assert.equal(requestAutoMergeResource.peek(identity)?.intent?.status, 'Stopped')
  assert.equal(requestAutoMergeResource.peek(identity)?.intent?.reason, 'ChecksFailed')
})

test('an action receipt cannot replace a newer event-triggered refresh', async () => {
  const identity = requestAutoMergeIdentity('viewer-access', 'request')
  requestAutoMergeResource.write(identity, status())
  const mutationGeneration = requestAutoMergeResource.invalidationGeneration(identity)
  invalidateRepoResources('viewer-access', {
    incarnation_id: 'incarnation',
    kind: { RepositoryChanged: { reason: 'request-auto-merge-updated' } },
    repo_id: 'repo',
    version: 2,
  })

  let finish!: (value: RequestAutoMergeResponse) => void
  let refreshSignal!: AbortSignal
  const refresh = requestAutoMergeResource.ensure(identity, '2', (signal) => {
    refreshSignal = signal
    return new Promise<RequestAutoMergeResponse>((resolve) => {
      finish = resolve
    })
  })
  await Promise.resolve()

  assert.equal(requestAutoMergeResource.writeIfNotInvalidated(
    identity,
    mutationGeneration,
    status(),
  ), false)
  assert.equal(refreshSignal.aborted, false)

  const fulfilled = status()
  fulfilled.can_cancel = false
  fulfilled.intent = { ...fulfilled.intent!, status: 'Fulfilled' }
  fulfilled.waiting_reason = null
  finish(fulfilled)
  await refresh

  assert.equal(requestAutoMergeResource.writeIfNotInvalidated(
    identity,
    mutationGeneration,
    status(),
  ), false)
  assert.equal(requestAutoMergeResource.peek(identity)?.intent?.status, 'Fulfilled')
})

function status(): RequestAutoMergeResponse {
  return {
    can_cancel: true,
    can_enable: false,
    head_oid: 'b'.repeat(40),
    intent: {
      actor: { handle: 'adam', id: 'user' },
      created_at_unix: 1,
      head_oid: 'b'.repeat(40),
      id: 'intent',
      reason: null,
      revision_id: 'revision',
      status: 'Active',
      updated_at_unix: 1,
    },
    request_id: 'request',
    revision_id: 'revision',
    waiting_reason: 'Checks are running.',
  }
}
