import assert from 'node:assert/strict'
import test from 'node:test'
import { requestChangesSelectionIdentity, requestChangesResource } from './request-changes-resource'

test('revision loading is deduplicated and reused while selected inspection inputs remain isolated', async () => {
  requestChangesResource.clear()
  const identity = requestChangesSelectionIdentity('viewer-access', 'request', 'revision', 'commit')
  let calls = 0
  const load = async () => {
    calls++
    return { revisions: [], has_earlier_revisions: false, review_revision_id: null }
  }
  await Promise.all([
    requestChangesResource.ensure(identity, '', load),
    requestChangesResource.ensure(identity, '', load),
  ])
  await requestChangesResource.ensure(identity, '', load)
  assert.equal(calls, 1)
  assert.equal(requestChangesResource.peek(requestChangesSelectionIdentity('viewer-access', 'request', 'revision', 'other-commit')), null)
  requestChangesResource.invalidate(identity)
  await requestChangesResource.ensure(identity, '', load)
  assert.equal(calls, 2)
})
