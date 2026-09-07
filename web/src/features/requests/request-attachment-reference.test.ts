import assert from 'node:assert/strict'
import test from 'node:test'
import {
  insertRequestAttachmentReferences,
  requestAttachmentDraftReference,
  requestAttachmentIdFromUrl,
} from './request-attachment-reference'

test('accepts only exact Scope attachment references', () => {
  assert.equal(requestAttachmentIdFromUrl('/request-attachments/photo-1'), 'photo-1')
  assert.equal(requestAttachmentIdFromUrl('https://evil.test/request-attachments/photo-1'), null)
  assert.equal(requestAttachmentIdFromUrl('/request-attachments/photo-1#fragment'), null)
  assert.equal(requestAttachmentIdFromUrl('/request-attachments/photo%2Fone'), null)
  assert.equal(requestAttachmentIdFromUrl('/other/photo-1'), null)
})

test('inserts a pending stable-shaped reference at the editor cursor', () => {
  const reference = requestAttachmentDraftReference({
    contentType: 'image/png', localId: 'local id', name: 'panel].png',
  })
  assert.equal(reference, '![panel\\].png](/request-attachments/local%20id)')
  assert.equal(
    insertRequestAttachmentReferences('before after', 6, [reference]),
    `before\n\n${reference}\n\n after`,
  )
})
