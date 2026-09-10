import assert from 'node:assert/strict'
import test from 'node:test'
import type { CreateRequestAttachmentMediaGrantResponse } from '../../api/types.generated'
import type { CachedResourceStore } from '../../lib/cached-resource'
import {
  activateRequestAttachmentMediaScope,
  requestAttachmentMediaGrantResource,
  resetRequestAttachmentMediaGrants,
} from './request-attachment-media-resource'
import {
  activateRequestAttachmentResourceScope,
  requestAttachmentResource,
  requestAttachmentResourceIdentity,
  resetRequestAttachmentResources,
  type RequestAttachmentResourceValue,
} from './request-attachment-resource'

const metadata: RequestAttachmentResourceValue = {
  attachments: [],
  limits: {
    accepted_photo_media_types: ['image/png'],
    accepted_video_media_types: ['video/quicktime'],
    incomplete_upload_ttl_seconds: 86_400,
    max_attachments_per_content: 10,
    max_concurrent_parts: 2,
    max_photo_bytes: 25,
    max_photo_pixels: 50,
    max_repository_storage_bytes: 20,
    max_request_source_bytes: 2,
    max_video_bytes: 500,
    max_video_duration_seconds: 600,
    preferred_part_bytes: 8,
    unbound_attachment_ttl_seconds: 604_800,
  },
}
const grant: CreateRequestAttachmentMediaGrantResponse = {
  media_url: '/media/attachment', grant: 'grant', expires_at_unix: 100,
}
const identity = requestAttachmentResourceIdentity
const mediaIdentity = (accessScope: string, attachmentId: string) =>
  `${accessScope}\0${attachmentId}\0${JSON.stringify({ kind: 'derivative', derivative_id: 'preview' })}`
const scope = (repo = 'repo', viewer = 'viewer', access = 'Public') =>
  JSON.stringify([repo, viewer, { actor: access }])

test.beforeEach(() => {
  resetRequestAttachmentResources()
  resetRequestAttachmentMediaGrants()
})

function characterize<T extends object>(
  name: string,
  store: CachedResourceStore<T>,
  activate: (scope: string) => void,
  reset: () => void,
  value: T,
  capacity: number,
  identity: (accessScope: string, itemId: string) => string,
) {
  test(`${name}: unchanged scope preserves cached values and navigation isolates repositories and viewers`, () => {
    const scopes = [scope(), scope('other-repo'), scope('repo', 'other-viewer')]
    for (const current of scopes) {
      activate(current)
      store.write(identity(current, 'request'), value)
    }
    activate(scopes[0]!)
    for (const current of scopes) assert.equal(store.peek(identity(current, 'request')), value)

    activate(scope('repo', 'viewer', 'Member'))
    assert.equal(store.peek(identity(scopes[0]!, 'request')), null)
    for (const current of scopes.slice(1)) {
      assert.equal(store.peek(identity(current, 'request')), value)
      assert.equal(store.getSnapshot(identity(current, 'request')).stale, false)
    }
  })

  test(`${name}: access changes delete all and only entries belonging to the previous scope`, () => {
    const previous = scope()
    const next = scope('repo', 'viewer', 'Member')
    const untracked = scope('repo', 'viewer', 'Owner')
    const retained = [identity(next, 'request'), identity(untracked, 'request'), `${previous}suffix\0request`]
    activate(previous)
    for (const key of [identity(previous, 'request'), identity(previous, 'other-request'), ...retained]) {
      store.write(key, value)
    }
    activate(next)
    assert.equal(store.peek(identity(previous, 'request')), null)
    assert.equal(store.peek(identity(previous, 'other-request')), null)
    for (const key of retained) assert.equal(store.peek(key), value)
  })

  test(`${name}: ${capacity}-owner tracking evicts oldest without deleting cached entries`, () => {
    // Activations and cache writes are separate so cache capacity cannot hide tracker eviction.
    for (let index = 0; index < capacity; index++) activate(scope(`repo-${index}`))
    store.write(identity(scope('repo-0'), 'request'), value)
    activate(scope('repo-0', 'viewer', 'Member'))
    assert.equal(store.peek(identity(scope('repo-0'), 'request')), null)
    reset()

    activate(scope('oldest'))
    store.write(identity(scope('oldest'), 'request'), value)
    for (let index = 1; index <= capacity; index++) activate(scope(`repo-${index}`))
    assert.equal(store.peek(identity(scope('oldest'), 'request')), value)
    activate(scope('oldest', 'viewer', 'Member'))
    assert.equal(store.peek(identity(scope('oldest'), 'request')), value)

    const newest = scope(`repo-${capacity}`)
    store.write(identity(newest, 'request'), value)
    activate(scope(`repo-${capacity}`, 'viewer', 'Member'))
    assert.equal(store.peek(identity(newest, 'request')), null)
  })

  test(`${name}: unchanged activation refreshes owner recency at capacity`, () => {
    for (let index = 0; index < capacity; index++) activate(scope(`repo-${index}`))
    activate(scope('repo-0'))
    activate(scope('overflow'))
    for (const repo of ['repo-0', 'repo-1']) store.write(identity(scope(repo), 'request'), value)
    activate(scope('repo-0', 'viewer', 'Member'))
    activate(scope('repo-1', 'viewer', 'Member'))
    assert.equal(store.peek(identity(scope('repo-0'), 'request')), null)
    assert.equal(store.peek(identity(scope('repo-1'), 'request')), value)
  })

  test(`${name}: malformed scopes neither delete cache nor consume tracker capacity`, () => {
    activate(scope())
    for (let index = 1; index < capacity; index++) activate(scope(`repo-${index}`))
    const invalid = ['{', 'null', '123', '{}', '"repo"', '[]', '[null]', '[1,"viewer"]']
    for (const current of invalid) {
      store.write(identity(current, 'request'), value)
      activate(current)
      assert.equal(store.peek(identity(current, 'request')), value)
    }
    store.write(identity(scope(), 'request'), value)
    activate(scope('repo', 'viewer', 'Member'))
    assert.equal(store.peek(identity(scope(), 'request')), null)
  })

  test(`${name}: partial tuples and non-string viewers retain the anonymous-owner fallback`, () => {
    const scopes = ['["repo"]', '["repo",null]', '["repo",12]', '["repo",{}]', '["repo",[]]', '["repo","anonymous"]']
    for (const current of scopes) {
      activate(current)
      store.write(identity(current, 'request'), value)
    }
    for (const previous of scopes.slice(0, -1)) assert.equal(store.peek(identity(previous, 'request')), null)
    assert.equal(store.peek(identity(scopes.at(-1)!, 'request')), value)
  })

  test(`${name}: reset clears cached values and forgets tracked owners`, () => {
    activate(scope())
    store.write(identity(scope(), 'request'), value)
    reset()
    assert.equal(store.peek(identity(scope(), 'request')), null)
    store.write(identity(scope(), 'request'), value)
    activate(scope('repo', 'viewer', 'Member'))
    assert.equal(store.peek(identity(scope(), 'request')), value)
  })

  test(`${name}: navigation reuses pending work and an access change cancels it`, async () => {
    const key = identity(scope(), 'request')
    activate(scope())
    let resolve!: (value: T) => void
    const response = new Promise<T>((complete) => { resolve = complete })
    let signal: AbortSignal | undefined
    const leave = store.subscribe(key, () => {})
    const first = store.ensure(key, '1', (nextSignal) => {
      signal = nextSignal
      return response
    })
    await Promise.resolve()
    leave()
    activate(scope('other-repo'))
    activate(scope())
    const leaveReturn = store.subscribe(key, () => {})
    try {
      const returning = store.ensure(key, '1', async () => assert.fail('must reuse pending request'))
      assert.equal(returning, first)
      assert.equal(signal?.aborted, false)
      activate(scope('repo', 'viewer', 'Member'))
      assert.equal(signal?.aborted, true)
      assert.equal(store.getSnapshot(key).pending, false)
      resolve(value)
      assert.equal(await first, null)
      assert.equal(store.peek(key), null)
    } finally {
      leaveReturn()
    }
  })

  test(`${name}: returning to a completed resource reuses cached data without loading`, async () => {
    const key = identity(scope(), 'request')
    activate(scope())
    const leave = store.subscribe(key, () => {})
    assert.equal(await store.ensure(key, '1', async () => value), value)
    leave()
    activate(scope('other-repo'))
    activate(scope())
    const leaveReturn = store.subscribe(key, () => {})
    try {
      assert.equal(await store.ensure(key, '1', async () => assert.fail('must reuse cached result')), value)
    } finally {
      leaveReturn()
    }
  })
}

characterize('attachment metadata', requestAttachmentResource, activateRequestAttachmentResourceScope,
  resetRequestAttachmentResources, metadata, 16, identity)
characterize('attachment media grants', requestAttachmentMediaGrantResource, activateRequestAttachmentMediaScope,
  resetRequestAttachmentMediaGrants, grant, 64, mediaIdentity)

test('attachment resource owners have independent activation and reset state', () => {
  const initial = scope()
  const next = scope('repo', 'viewer', 'Member')
  const key = identity(initial, 'request')
  const mediaKey = mediaIdentity(initial, 'attachment')
  activateRequestAttachmentResourceScope(initial)
  activateRequestAttachmentMediaScope(initial)
  requestAttachmentResource.write(key, metadata)
  requestAttachmentMediaGrantResource.write(mediaKey, grant)
  activateRequestAttachmentResourceScope(next)
  assert.equal(requestAttachmentResource.peek(key), null)
  assert.equal(requestAttachmentMediaGrantResource.peek(mediaKey), grant)

  resetRequestAttachmentResources()
  activateRequestAttachmentMediaScope(next)
  assert.equal(requestAttachmentMediaGrantResource.peek(mediaKey), null)

  activateRequestAttachmentResourceScope(initial)
  requestAttachmentResource.write(key, metadata)
  resetRequestAttachmentMediaGrants()
  assert.equal(requestAttachmentResource.peek(key), metadata)
  activateRequestAttachmentResourceScope(next)
  assert.equal(requestAttachmentResource.peek(key), null)
})
