import assert from 'node:assert/strict'
import test from 'node:test'
import {
  activateRequestAttachmentResourceScope,
  requestAttachmentResource,
  requestAttachmentResourceIdentity,
  resetRequestAttachmentResources,
} from './request-attachment-resource'
import type { RequestAttachmentResourceValue } from './request-attachment-resource'

test.beforeEach(resetRequestAttachmentResources)

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

test('navigation retains other repositories while an access change invalidates that repository', () => {
  const viewer = 'viewer'
  const repoOnePublic = JSON.stringify(['repo-one', viewer, { actor: 'Public' }])
  const repoOnePrivate = JSON.stringify(['repo-one', viewer, { actor: 'Member' }])
  const repoTwo = JSON.stringify(['repo-two', viewer, { actor: 'Member' }])
  const repoOneIdentity = requestAttachmentResourceIdentity(repoOnePublic, 'request-one')
  const repoTwoIdentity = requestAttachmentResourceIdentity(repoTwo, 'request-two')
  requestAttachmentResource.write(repoOneIdentity, resourceValue('Processing'))
  requestAttachmentResource.write(repoTwoIdentity, resourceValue('Processing'))

  activateRequestAttachmentResourceScope(repoOnePublic)
  activateRequestAttachmentResourceScope(repoTwo)
  assert.equal(requestAttachmentResource.peek(repoOneIdentity) !== null, true)
  assert.equal(requestAttachmentResource.peek(repoTwoIdentity) !== null, true)

  activateRequestAttachmentResourceScope(repoOnePrivate)
  assert.equal(requestAttachmentResource.peek(repoOneIdentity), null)
  assert.equal(requestAttachmentResource.getSnapshot(repoTwoIdentity).stale, false)
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
