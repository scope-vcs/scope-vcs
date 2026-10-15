import assert from 'node:assert/strict'
import test from 'node:test'
import { resetViewerState } from '../../lib/viewer-state'
import { invalidateRepoResources } from '../repo-detail/repo-resource-invalidation'
import {
  activateRequestAttachmentResourceScope as activate,
  requestAttachmentMediaGrantResource,
  requestAttachmentResource,
  requestAttachmentResourceIdentity,
} from './request-attachment-resource'
import type { RequestAttachmentResourceValue } from './request-attachment-resource'

test.beforeEach(resetViewerState)

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

const scope = (repo = 'repo', viewer = 'viewer', access = 'Public') =>
  JSON.stringify([repo, viewer, { actor: access }])
const maxOwners = 64
const seedMetadata = (key: string) => requestAttachmentResource.write(key, resourceValue('Processing'))
const seedScope = (accessScope: string) => seedMetadata(requestAttachmentResourceIdentity(accessScope, 'request'))
const peekScope = (accessScope: string) => requestAttachmentResource.peek(requestAttachmentResourceIdentity(accessScope, 'request'))
const owners = [
  {
    name: 'metadata', resource: requestAttachmentResource,
    identity: requestAttachmentResourceIdentity,
    seed: seedMetadata,
  },
  {
    name: 'media grants', resource: requestAttachmentMediaGrantResource,
    identity: (scope: string, id: string) => `${scope}\0${id}\0${JSON.stringify({ kind: 'derivative', derivative_id: 'preview' })}`,
    seed: (key: string) => requestAttachmentMediaGrantResource.write(key, {
      media_url: '/media/attachment', grant: 'grant', expires_at_unix: 100,
    }),
  },
]

for (const owner of owners) {
  const { resource, seed } = owner
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
}

test(`${maxOwners}-owner tracking refreshes recency and forgets without cache deletion`, () => {
  for (let index = 0; index < maxOwners; index++) activate(scope(`repo-${index}`))
  seedScope(scope('repo-0'))
  activate(scope('repo-0', 'viewer', 'Member'))
  assert.equal(peekScope(scope('repo-0')), null)
  activate(scope('repo-1'))
  for (const repo of ['repo-1', 'repo-2']) seedScope(scope(repo))
  activate(scope('overflow'))
  assert.notEqual(peekScope(scope('repo-2')), null)
  activate(scope('repo-1', 'viewer', 'Member'))
  activate(scope('repo-2', 'viewer', 'Member'))
  assert.equal(peekScope(scope('repo-1')), null)
  assert.notEqual(peekScope(scope('repo-2')), null)
})

test('decoding ignores malformed scopes and preserves the anonymous fallback', () => {
  activate(scope())
  for (let index = 1; index < maxOwners; index++) activate(scope(`repo-${index}`))
  for (const invalid of ['{', 'null', '123', '{}', '"repo"', '[]', '[null]', '[1,"viewer"]']) {
    seedScope(invalid)
    activate(invalid)
    assert.notEqual(peekScope(invalid), null)
  }
  seedScope(scope())
  activate(scope('repo', 'viewer', 'Member'))
  assert.equal(peekScope(scope()), null)
  const anonymous = ['["repo"]', '["repo",null]', '["repo",12]', '["repo",{}]', '["repo",[]]', '["repo","anonymous"]']
  for (const current of anonymous) {
    activate(current)
    seedScope(current)
  }
  for (const previous of anonymous.slice(0, -1)) assert.equal(peekScope(previous), null)
  assert.notEqual(peekScope(anonymous.at(-1)!), null)
})

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
  { RequestAttachmentChanged: { request_id: 'request', attachment_id: 'attachment', view: 'public' as const } },
  { RequestTimelineChanged: { request_id: 'request', discussion_id: 'discussion', through_position: 2, view: 'public' as const } },
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
