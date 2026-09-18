import assert from 'node:assert/strict'
import test from 'node:test'
import type { RequestChecksResponse } from '@/api/types.generated'
import { invalidateRepoResources } from '../repo-detail/repo-resource-invalidation'
import {
  requestChecksIdentity,
  requestChecksResource,
} from './request-checks-resource'

test.beforeEach(() => requestChecksResource.clear())

test('one evaluation loads once and is reused by later mounts of the same request', async () => {
  const identity = requestChecksIdentity('viewer-access', 'request')
  let calls = 0
  const load = async () => {
    calls++
    return checks('started', 'running')
  }

  await Promise.all([
    requestChecksResource.ensure(identity, '', load),
    requestChecksResource.ensure(identity, '', load),
  ])
  await requestChecksResource.ensure(identity, '', load)

  assert.equal(calls, 1)
  assert.equal(requestChecksResource.peek(identity)?.state, 'started')
  // Another viewer, access scope or request never reads this entry.
  for (const other of [
    requestChecksIdentity('other-access', 'request'),
    requestChecksIdentity('viewer-access', 'other-request'),
  ]) {
    assert.equal(requestChecksResource.peek(other), null)
  }
})

test('approval writes the refreshed evaluation without another read', async () => {
  const identity = requestChecksIdentity('viewer-access', 'request')
  let calls = 0
  const load = async () => {
    calls++
    return checks('awaiting-approval', null)
  }
  await requestChecksResource.ensure(identity, '', load)

  requestChecksResource.write(identity, checks('started', 'queued'))
  await requestChecksResource.ensure(identity, '', load)

  assert.equal(calls, 1)
  assert.equal(requestChecksResource.peek(identity)?.state, 'started')
  assert.equal(requestChecksResource.peek(identity)?.checks[0]?.run_state, 'queued')
})

for (const kind of [
  { RunChanged: { run_id: 'run', change: 'StatusChanged' as const } },
  { RepositoryChanged: { reason: 'request-revised' } },
  'Lagged' as const,
]) {
  test(`checks refresh without blanking valid rows: ${JSON.stringify(kind)}`, async () => {
    const identity = requestChecksIdentity('viewer-access', 'request')
    const other = requestChecksIdentity('other-access', 'request')
    requestChecksResource.write(identity, checks('started', 'running'))
    requestChecksResource.write(other, checks('started', 'running'))

    invalidateRepoResources('viewer-access', {
      incarnation_id: 'incarnation',
      kind,
      repo_id: 'repo',
      version: 2,
    })

    assert.equal(requestChecksResource.getSnapshot(identity).stale, true)
    assert.equal(requestChecksResource.peek(identity)?.checks[0]?.run_state, 'running')
    assert.equal(requestChecksResource.getSnapshot(other).stale, false)

    // A failed refresh keeps showing the rows it still has.
    const refresh = requestChecksResource.ensure(identity, '2', async () => {
      throw new Error('temporary outage')
    })
    assert.equal(requestChecksResource.peek(identity)?.checks[0]?.run_state, 'running')
    await refresh
    assert.equal(requestChecksResource.peek(identity)?.checks[0]?.run_state, 'running')
    assert.equal(requestChecksResource.getSnapshot(identity).error instanceof Error, true)
  })
}

function checks(
  state: RequestChecksResponse['state'],
  runState: RequestChecksResponse['checks'][number]['run_state'],
): RequestChecksResponse {
  return {
    can_approve: state === 'awaiting-approval',
    checks: [{
      run_id: runState ? 'run' : null,
      run_state: runState,
      workflow_name: 'checks',
      workflow_path: '/.scope/runs/checks.yml',
    }],
    head_oid: 'b'.repeat(40),
    mergeability: {
      current_main_oid: 'a'.repeat(40),
      reason: 'checks have not finished',
      request_head_oid: 'b'.repeat(40),
      status: 'ChecksPending',
    },
    message: null,
    request_id: 'request',
    state,
  }
}
