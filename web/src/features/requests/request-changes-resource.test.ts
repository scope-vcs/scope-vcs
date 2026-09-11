import assert from 'node:assert/strict'
import test from 'node:test'
import { loadMoreDiscussionReferences, requestChangesIdentity, requestChangesSelectionIdentity, requestChangesResource, requestDiscussionReferenceIdentity, requestDiscussionReferenceResource as resource } from './request-changes-resource'
import type { RequestDiscussion, RequestDiscussionPage } from './request-discussion-types'

const page = (id: string, cursor: string | null, version = 1): RequestDiscussionPage => ({
  discussions: [{ id } as RequestDiscussion], next_cursor: cursor, snapshot_version: version,
})

test('reference pages survive reopening and equivalent initial loads, isolated by viewer and request', async () => {
  resource.clear()
  const identity = requestDiscussionReferenceIdentity('viewer-a', 'request', 'revision:commit')
  let loads = 0
  const first = () => { loads++; return Promise.resolve(page('first', 'next')) }
  await resource.ensure(identity, '', first)
  await loadMoreDiscussionReferences(identity, async cursor => {
    assert.equal(cursor, 'next')
    return page('second', null)
  })
  const unsubscribe = resource.subscribe(identity, () => {})
  unsubscribe()
  await resource.ensure(identity, '', first)
  assert.equal(loads, 1)
  assert.deepEqual(resource.peek(identity)?.discussions.map(item => item.id), ['first', 'second'])
  assert.equal(resource.peek(requestDiscussionReferenceIdentity('viewer-b', 'request', 'revision:commit')), null)
  assert.equal(resource.peek(requestDiscussionReferenceIdentity('viewer-a', 'other', 'revision:commit')), null)
})

test('failed snapshot continuation keeps visible data and first-page retry replaces expired cursor', async () => {
  resource.clear()
  const identity = requestDiscussionReferenceIdentity('scope', 'request', 'commit')
  resource.write(identity, page('old', 'expired'))
  await loadMoreDiscussionReferences(identity, async () => page('new', 'fresh', 2))
  assert.match(String(resource.getSnapshot(identity).error), /Discussions changed/)
  assert.equal(resource.peek(identity)?.discussions[0]?.id, 'old')
  resource.invalidate(identity)
  await resource.ensure(identity, '', async () => page('new', 'fresh', 2))
  assert.equal(resource.peek(identity)?.next_cursor, 'fresh')
  await loadMoreDiscussionReferences(identity, async cursor => {
    assert.equal(cursor, 'fresh')
    return page('new-last', null, 2)
  })
  assert.deepEqual(resource.peek(identity)?.discussions.map(item => item.id), ['new', 'new-last'])
})

test('request invalidation refreshes first page without blanking valid references', async () => {
  resource.clear()
  const identity = requestDiscussionReferenceIdentity('scope', 'request', 'commit')
  resource.write(identity, page('old', 'old-cursor'))
  resource.invalidateMatching(key => key.startsWith(`${requestChangesIdentity('scope', 'request')}\0`))
  assert.equal(resource.peek(identity)?.discussions[0]?.id, 'old')
  await resource.ensure(identity, '', async () => page('updated', null, 2))
  assert.equal(resource.peek(identity)?.discussions[0]?.id, 'updated')
})

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
