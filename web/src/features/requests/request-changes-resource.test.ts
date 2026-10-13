import assert from 'node:assert/strict'
import test from 'node:test'
import { requestChangesSelectionIdentity, requestChangesResource } from './request-changes-resource'

test('revision loading is deduplicated and reused while selected inspection inputs remain isolated', async () => {
  requestChangesResource.clear()
  const identity = requestChangesSelectionIdentity('viewer-access', 'request', 'revision', 'commit')
  let calls = 0
  const load = async () => {
    calls++
    return { revisions: [], has_earlier_revisions: false, review_revision_id: null, comparison: null }
  }
  await Promise.all([
    requestChangesResource.ensure(identity, '', load),
    requestChangesResource.ensure(identity, '', load),
  ])
  await requestChangesResource.ensure(identity, '', load)
  assert.equal(calls, 1)
  assert.equal(requestChangesResource.peek(requestChangesSelectionIdentity('viewer-access', 'request', 'revision', 'other-commit')), null)
  assert.equal(requestChangesResource.peek(requestChangesSelectionIdentity('other-viewer-access', 'request', 'revision', 'commit')), null)
  requestChangesResource.invalidate(identity)
  await requestChangesResource.ensure(identity, '', load)
  assert.equal(calls, 2)
})

test('request and push comparisons reopen independently for the same immutable revision', async () => {
  requestChangesResource.clear()
  const requestIdentity = requestChangesSelectionIdentity('viewer-access', 'request', 'revision', undefined, 'request')
  const pushIdentity = requestChangesSelectionIdentity('viewer-access', 'request', 'revision', undefined, 'push')
  const request = { revisions: [], has_earlier_revisions: false, review_revision_id: 'revision', comparison: null }
  const push = { ...request, has_earlier_revisions: true }
  let requestCalls = 0
  let pushCalls = 0
  const loadRequest = async () => { requestCalls++; return request }
  const loadPush = async () => { pushCalls++; return push }
  await requestChangesResource.ensure(requestIdentity, '', loadRequest)
  await requestChangesResource.ensure(pushIdentity, '', loadPush)
  await requestChangesResource.ensure(requestIdentity, '', loadRequest)
  await requestChangesResource.ensure(pushIdentity, '', loadPush)
  assert.equal(requestCalls, 1)
  assert.equal(pushCalls, 1)
  assert.equal(requestChangesResource.peek(requestIdentity)?.has_earlier_revisions, false)
  assert.equal(requestChangesResource.peek(pushIdentity)?.has_earlier_revisions, true)
})
