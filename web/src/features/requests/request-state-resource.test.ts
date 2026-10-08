import assert from 'node:assert/strict'
import test, { mock } from 'node:test'
import type { RepoChangeEvent, RequestStateResponse } from '@/api/types.generated'
import { invalidateRepoResources } from '../repo-detail/repo-resource-invalidation'
import { resetViewerState } from '../../lib/viewer-state'
import { deferred } from './request-discussion-test-fixtures'
import { loadRequestStateValue, reconcileRequestState, requestStateIdentity, requestStateResource } from './request-state-resource'

const identity = requestStateIdentity('viewer-access', 'request')
const event = (kind: RepoChangeEvent['kind']): RepoChangeEvent => ({ repo_id: 'repo', incarnation_id: 'incarnation', kind, version: 2 })
test.beforeEach(() => requestStateResource.clear())

test('navigation and reopening share the complete request snapshot in the viewer access scope', async () => {
  const load = mock.fn(async () => ({ state: state() }))
  await Promise.all([loadRequestStateValue(identity, load), loadRequestStateValue(identity, load)])
  await loadRequestStateValue(identity, load)
  assert.equal(load.mock.callCount(), 1)
  assert.equal(requestStateResource.peek(identity)?.state?.checks.head_oid, 'head')
  assert.equal(requestStateResource.peek(requestStateIdentity('other-access', 'request')), null)
  requestStateResource.invalidate(identity)
  const retained = await loadRequestStateValue(identity, async () => { throw new Error('offline') })
  assert.equal(retained.state?.detail.request.head_oid, 'head')
  assert.equal((requestStateResource.getSnapshot(identity).error as Error).message, 'offline')
})

for (const kind of [
  { RequestStateChanged: { request_id: 'request', view: 'public' } },
  { RequestTimelineChanged: { request_id: 'request', view: 'public', discussion_id: 'discussion', through_position: 2 } },
] satisfies RepoChangeEvent['kind'][]) {
  test(`a request event refreshes its retained snapshot only: ${JSON.stringify(kind)}`, () => {
    const other = requestStateIdentity('viewer-access', 'other')
    requestStateResource.write(identity, { state: state() })
    requestStateResource.write(other, { state: state() })
    invalidateRepoResources('viewer-access', event(kind))
    assert.equal(requestStateResource.getSnapshot(identity).stale, true)
    assert.equal(requestStateResource.peek(identity)?.state?.auto_merge.can_cancel, true)
    assert.equal(requestStateResource.getSnapshot(other).stale, false)
  })
}

test('a workflow run association arriving after check results refreshes embedded check links', () => {
  requestStateResource.write(identity, { state: state() })
  invalidateRepoResources('viewer-access', event('GitHubWorkflowRunsChanged'))
  assert.equal(requestStateResource.getSnapshot(identity).stale, true)
  assert.equal(requestStateResource.peek(identity)?.state?.checks.checks[0]?.provider, 'native')
})

test('native run changes leave requests with different runs reusable', () => {
  const other = requestStateIdentity('viewer-access', 'other')
  requestStateResource.write(identity, { state: state() })
  const otherState = state()
  otherState.checks.checks = [{ provider: 'native', run_id: 'other-run', run_state: 'running', workflow_name: 'checks', workflow_path: '/.scope/runs/checks.yml' }]
  requestStateResource.write(other, { state: otherState })
  invalidateRepoResources('viewer-access', event({ RunChanged: { run_id: 'run', change: 'StatusChanged' } }))
  assert.equal(requestStateResource.getSnapshot(identity).stale, true)
  assert.equal(requestStateResource.getSnapshot(other).stale, false)
})

for (const retained of [false, true]) {
  test(`a native completion during a pending snapshot load triggers recovery with retained data: ${retained}`, async () => {
    if (retained) requestStateResource.write(identity, { state: state() })
    requestStateResource.invalidate(identity)
    const response = deferred<{ state: RequestStateResponse }>()
    const first = loadRequestStateValue(identity, () => response.promise)
    await Promise.resolve()
    invalidateRepoResources('viewer-access', event({ RunChanged: { run_id: 'new-run', change: 'StatusChanged' } }))
    const incoming = state()
    incoming.checks.checks = [{ provider: 'native', run_id: 'new-run', run_state: 'running', workflow_name: 'checks', workflow_path: '/.scope/runs/checks.yml' }]
    response.resolve({ state: incoming })
    await first
    assert.equal(requestStateResource.getSnapshot(identity).stale, true)
    const recovered = state()
    recovered.checks.checks = [{ provider: 'native', run_id: 'new-run', run_state: 'succeeded', workflow_name: 'checks', workflow_path: '/.scope/runs/checks.yml' }]
    await loadRequestStateValue(identity, async () => ({ state: recovered }))
    const check = requestStateResource.peek(identity)?.state?.checks.checks[0]
    assert.equal(check?.provider === 'native' ? check.run_state : null, 'succeeded')
    assert.equal(requestStateResource.getSnapshot(identity).stale, false)
  })
}

test('a mutation receipt cannot mix requests or heads or replace an event-triggered snapshot', async () => {
  requestStateResource.write(identity, { state: state() })
  const receipt = requestStateResource.getSnapshot(identity)
  assert.equal(reconcileRequestState(identity, receipt, (current) => ({ ...current, auto_merge: { ...current.auto_merge, head_oid: 'other' } })), false)
  assert.equal(reconcileRequestState(identity, receipt, (current) => ({ ...current, auto_merge: { ...current.auto_merge, request_id: 'other' } })), false)
  invalidateRepoResources('viewer-access', event({ RequestStateChanged: { request_id: 'request', view: 'public' } }))
  await loadRequestStateValue(identity, async () => ({ state: state('new-head') }))
  assert.equal(reconcileRequestState(identity, receipt, (current) => ({ ...current, auto_merge: { ...current.auto_merge, can_cancel: false } })), false)
  assert.equal(requestStateResource.peek(identity)?.state?.detail.request.head_oid, 'new-head')
})

test('viewer reset prevents a late navigation response from publishing retained data', async () => {
  const response = deferred<{ state: RequestStateResponse }>()
  const load = loadRequestStateValue(identity, () => response.promise)
  await Promise.resolve()
  resetViewerState()
  response.resolve({ state: state() })
  await assert.rejects(load, /Resource is no longer available/)
  assert.equal(requestStateResource.peek(identity), null)
})

function state(head = 'head'): RequestStateResponse {
  const mergeability = { status: 'ChecksPending' as const, current_main_oid: 'main', request_head_oid: head, reason: 'checks have not finished' }
  return {
    detail: { request: { id: 'request', head_oid: head, mergeability } },
    checks: {
      request_id: 'request', head_oid: head, state: 'started', message: null,
      checks: [{ provider: 'native', run_id: 'run', run_state: 'running', workflow_name: 'checks', workflow_path: '/.scope/runs/checks.yml' }],
      can_approve: false, changes_github_workflows: false, private_request_on_public_github: false, github_push: null, mergeability,
    },
    auto_merge: { request_id: 'request', revision_id: 'revision', head_oid: head, intent: { id: 'intent', actor: { id: 'user', handle: 'adam' }, revision_id: 'revision', head_oid: head, status: 'Active', reason: null, created_at_unix: 1, updated_at_unix: 1 }, waiting_reason: 'Checks are running.', can_enable: false, can_cancel: true },
  } as RequestStateResponse
}
