import assert from 'node:assert/strict'
import test from 'node:test'
import { invalidateRepoResources } from '../repo-detail/repo-resource-invalidation'
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
} from './request-attachment-resource'
import type { RequestAttachmentResourceValue } from './request-attachment-resource'

test.beforeEach(() => {
  resetRequestAttachmentResources()
  resetRequestAttachmentMediaGrants()
})

test('attachment resources are isolated by viewer/access scope and request', () => {
  assert.notEqual(
    requestAttachmentResourceIdentity('viewer/private', 'request'),
    requestAttachmentResourceIdentity('viewer/public', 'request'),
  )
  assert.notEqual(
    requestAttachmentResourceIdentity('viewer/private', 'request'),
    requestAttachmentResourceIdentity('viewer/private', 'other'),
  )
})

test('invalidation retains valid metadata while a processing refresh runs or fails', async () => {
  const identity = requestAttachmentResourceIdentity('viewer/private', 'request')
  const current = resourceValue('Processing')
  requestAttachmentResource.write(identity, current)
  requestAttachmentResource.invalidate(identity)

  assert.equal(requestAttachmentResource.getSnapshot(identity).value?.attachments[0]?.state, 'Processing')
  const refresh = requestAttachmentResource.ensure(identity, '2', async () => {
    throw new Error('temporary outage')
  })
  assert.equal(requestAttachmentResource.getSnapshot(identity).value?.attachments[0]?.state, 'Processing')
  await refresh
  assert.equal(requestAttachmentResource.getSnapshot(identity).value?.attachments[0]?.state, 'Processing')
  assert.equal(requestAttachmentResource.getSnapshot(identity).error instanceof Error, true)
})

const scope = (repo = 'repo', viewer = 'viewer', access = 'Public') =>
  JSON.stringify([repo, viewer, { actor: access }])
const owners = [
  {
    name: 'metadata', resource: requestAttachmentResource, maxOwners: 16,
    activate: activateRequestAttachmentResourceScope, reset: resetRequestAttachmentResources,
    identity: requestAttachmentResourceIdentity,
    seed: (key: string) => requestAttachmentResource.write(key, resourceValue('Processing')),
  },
  {
    name: 'media grants', resource: requestAttachmentMediaGrantResource, maxOwners: 64,
    activate: activateRequestAttachmentMediaScope, reset: resetRequestAttachmentMediaGrants,
    identity: (scope: string, id: string) => `${scope}\0${id}\0${JSON.stringify({ kind: 'derivative', derivative_id: 'preview' })}`,
    seed: (key: string) => requestAttachmentMediaGrantResource.write(key, {
      media_url: '/media/attachment', grant: 'grant', expires_at_unix: 100,
    }),
  },
]

for (const owner of owners) {
  const { resource, maxOwners, activate, reset, seed } = owner
  const key = (accessScope: string, id = 'request') => owner.identity(accessScope, id)

  test(`${owner.name}: scope reuse retains data and changes remove only the previous scope`, () => {
    const previous = scope()
    const next = scope('repo', 'viewer', 'Member')
    const removed = [key(previous), key(previous, 'other-request')]
    const retained = [key(next), key(scope('other-repo')), key(scope('repo', 'other-viewer')),
      key(scope('repo', 'viewer', 'Owner')), `${previous}suffix\0request`]
    activate(previous)
    for (const identity of [...removed, ...retained]) seed(identity)
    const snapshot = resource.getSnapshot(removed[0]!)
    activate(scope('other-repo'))
    activate(scope('repo', 'other-viewer'))
    activate(previous)
    assert.equal(resource.getSnapshot(removed[0]!), snapshot)
    activate(next)
    for (const identity of removed) assert.equal(resource.peek(identity), null)
    for (const identity of retained) assert.equal(resource.getSnapshot(identity).stale, false)
  })

  test(`${owner.name}: ${maxOwners}-owner tracking refreshes recency and forgets without cache deletion`, () => {
    // Populate tracking separately so cache eviction cannot mask its capacity or recency.
    for (let index = 0; index < maxOwners; index++) activate(scope(`repo-${index}`))
    seed(key(scope('repo-0')))
    activate(scope('repo-0', 'viewer', 'Member'))
    assert.equal(resource.peek(key(scope('repo-0'))), null)
    activate(scope('repo-1'))
    for (const repo of ['repo-1', 'repo-2']) seed(key(scope(repo)))
    activate(scope('overflow'))
    assert.notEqual(resource.peek(key(scope('repo-2'))), null)
    activate(scope('repo-1', 'viewer', 'Member'))
    activate(scope('repo-2', 'viewer', 'Member'))
    assert.equal(resource.peek(key(scope('repo-1'))), null)
    assert.notEqual(resource.peek(key(scope('repo-2'))), null)
  })

  test(`${owner.name}: decoding ignores malformed scopes and preserves the anonymous fallback`, () => {
    activate(scope())
    for (let index = 1; index < maxOwners; index++) activate(scope(`repo-${index}`))
    for (const invalid of ['{', 'null', '123', '{}', '"repo"', '[]', '[null]', '[1,"viewer"]']) {
      seed(key(invalid))
      activate(invalid)
      assert.notEqual(resource.peek(key(invalid)), null)
    }
    seed(key(scope()))
    activate(scope('repo', 'viewer', 'Member'))
    assert.equal(resource.peek(key(scope())), null)
    const anonymous = ['["repo"]', '["repo",null]', '["repo",12]', '["repo",{}]', '["repo",[]]', '["repo","anonymous"]']
    for (const current of anonymous) {
      activate(current)
      seed(key(current))
    }
    for (const previous of anonymous.slice(0, -1)) assert.equal(resource.peek(key(previous)), null)
    assert.notEqual(resource.peek(key(anonymous.at(-1)!)), null)
  })

  test(`${owner.name}: activation and reset leave the other resource's state alone`, () => {
    const other = owners.find((candidate) => candidate !== owner)!
    const previous = scope()
    const next = scope('repo', 'viewer', 'Member')
    const otherKey = other.identity(previous, 'request')
    activate(previous)
    other.activate(previous)
    seed(key(previous))
    other.seed(otherKey)
    activate(next)
    assert.equal(resource.peek(key(previous)), null)
    assert.notEqual(other.resource.peek(otherKey), null)
    seed(key(next))
    reset()
    assert.equal(resource.peek(key(next)), null)
    assert.notEqual(other.resource.peek(otherKey), null)
    seed(key(previous))
    activate(next)
    assert.notEqual(resource.peek(key(previous)), null)
    other.activate(next)
    assert.equal(other.resource.peek(otherKey), null)
  })
}

function resourceValue(state: 'Processing'): RequestAttachmentResourceValue {
  return {
    attachments: [{
      created_at_unix: 1,
      declared_media_type: 'video/quicktime',
      derivatives: [],
      detected_media_type: 'video/quicktime',
      failure: null,
      filename: 'walkthrough.mov',
      id: 'attachment',
      image: null,
      kind: 'Video',
      original_download_available: true,
      request_id: 'request',
      sha256: 'a'.repeat(64),
      size_bytes: 42,
      state,
      updated_at_unix: 1,
      uploader_user_id: 'viewer',
      video: null,
    }],
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
}

for (const kind of [
  { RequestAttachmentChanged: { request_id: 'request', attachment_id: 'attachment', audience: 'Public' as const } },
  { RequestTimelineChanged: { request_id: 'request', discussion_id: 'discussion', through_position: 2, audience: 'Public' as const } },
  'Lagged' as const,
  { RepositoryChanged: { reason: 'recovery' } },
]) {
  test(`repository events invalidate an unmounted attachment cache: ${JSON.stringify(kind)}`, () => {
    const identity = requestAttachmentResourceIdentity('viewer/private', 'request')
    const other = requestAttachmentResourceIdentity('other-viewer', 'request')
    requestAttachmentResource.write(identity, resourceValue('Processing'))
    requestAttachmentResource.write(other, resourceValue('Processing'))
    invalidateRepoResources('viewer/private', { repo_id: 'repo', incarnation_id: 'incarnation', version: 2, kind })
    assert.equal(requestAttachmentResource.getSnapshot(identity).stale, true)
    assert.equal(requestAttachmentResource.peek(identity)?.attachments[0]?.state, 'Processing')
    assert.equal(requestAttachmentResource.getSnapshot(other).stale, false)
  })
}
