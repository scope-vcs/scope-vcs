import assert from 'node:assert/strict'
import test from 'node:test'
import { requestChangesSelectionIdentity, requestChangesResource, requestChangesQuery } from './request-changes-resource'

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

test('push file selections share revision inspection without sending commit filters', async () => {
  requestChangesResource.clear()
  const response = { revisions: [], has_earlier_revisions: false, review_revision_id: 'revision', comparison: null }
  const queries: ReturnType<typeof requestChangesQuery>[] = []
  for (const search of [
    { revision: 'revision' },
    { revision: 'revision', path: '/src/retry.ts' },
    { revision: 'revision', comparison: 'push' as const, commit: 'visible-commit' },
  ]) {
    const query = requestChangesQuery(search, 'push')
    const identity = requestChangesSelectionIdentity('viewer-access', 'request', query.revision_id, query.commit_oid, query.comparison)
    await requestChangesResource.ensure(identity, '', async () => {
      queries.push(query)
      return response
    })
  }
  assert.deepEqual(queries, [{ comparison: 'push', commit_oid: undefined, revision_id: 'revision' }])
  assert.deepEqual(requestChangesQuery({ revision: 'revision' }), { comparison: undefined, commit_oid: undefined, revision_id: 'revision' })
  const privateQuery = requestChangesQuery({ revision: 'revision' }, 'request')
  assert.deepEqual(privateQuery, { comparison: 'request', commit_oid: undefined, revision_id: 'revision' })
  const commitQuery = requestChangesQuery({ revision: 'revision', comparison: 'commit', commit: 'visible-commit' }, 'push')
  assert.deepEqual(commitQuery, { comparison: 'commit', commit_oid: 'visible-commit', revision_id: 'revision' })
  assert.equal(requestChangesResource.peek(requestChangesSelectionIdentity('viewer-access', 'request', commitQuery.revision_id, commitQuery.commit_oid, commitQuery.comparison)), null)
})
