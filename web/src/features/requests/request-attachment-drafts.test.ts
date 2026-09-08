import assert from 'node:assert/strict'
import test from 'node:test'
import {
  activateRequestAttachmentDraftScope,
  addRequestAttachmentDraftFiles,
  clearRequestAttachmentDraft,
  patchRequestAttachmentDraftFile,
  readRequestAttachmentDraft,
  requestAttachmentDraftKey,
  requestAttachmentMarkdownReference,
  registerRequestAttachmentDraftUploadCanceller,
  resetRequestAttachmentDraftManager,
  setRequestAttachmentDraftText,
} from './request-attachment-drafts'

test.beforeEach(resetRequestAttachmentDraftManager)

test('drafts survive reopen and remain isolated by viewer, access, request, and target', () => {
  const base = {
    accessScope: 'maintainer-private',
    repoId: 'repo',
    requestId: 'request',
    target: 'discussion' as const,
    viewerId: 'ravi',
  }
  const key = requestAttachmentDraftKey(base)
  setRequestAttachmentDraftText(key, 'kept across route mounts')

  assert.equal(readRequestAttachmentDraft(requestAttachmentDraftKey({ ...base })).text, 'kept across route mounts')
  assert.equal(readRequestAttachmentDraft(requestAttachmentDraftKey({ ...base, viewerId: 'alex' })).text, '')
  assert.equal(readRequestAttachmentDraft(requestAttachmentDraftKey({ ...base, accessScope: 'public' })).text, '')
  assert.equal(readRequestAttachmentDraft(requestAttachmentDraftKey({ ...base, requestId: 'other' })).text, '')
  assert.equal(readRequestAttachmentDraft(requestAttachmentDraftKey({ ...base, target: 'reply:one' })).text, '')
})

test('an uploaded file keeps its stable markdown reference until the draft is cleared', () => {
  const key = 'draft'
  const [attachment] = addRequestAttachmentDraftFiles(
    key,
    [new File(['image'], 'screen.png', { type: 'image/png' })],
  )
  assert.ok(attachment)
  patchRequestAttachmentDraftFile(key, attachment.localId, {
    attachmentId: 'attachment / one',
    progress: 1,
    status: 'uploaded',
  })

  assert.equal(readRequestAttachmentDraft(key).attachments[0]?.attachmentId, 'attachment / one')
  assert.equal(requestAttachmentMarkdownReference('attachment / one'), '/request-attachments/attachment%20%2F%20one')

  clearRequestAttachmentDraft(key)
  assert.deepEqual(readRequestAttachmentDraft(key), { attachments: [], baseText: null, initialized: false, text: '' })
})

test('session restore keeps text, completed ids, and an incomplete operation for reselection', () => {
  const original = globalThis.sessionStorage
  const values = new Map<string, string>()
  Object.defineProperty(globalThis, 'sessionStorage', {
    configurable: true,
    value: {
      getItem: (key: string) => values.get(key) ?? null,
      key: (index: number) => [...values.keys()][index] ?? null,
      get length() { return values.size },
      removeItem: (key: string) => values.delete(key),
      setItem: (key: string, value: string) => values.set(key, value),
    },
  })
  try {
    const key = requestAttachmentDraftKey({
      accessScope: 'private', repoId: 'repo', requestId: 'request', target: 'discussion', viewerId: 'ravi',
    })
    setRequestAttachmentDraftText(key, 'full reload draft')
    const [pending] = addRequestAttachmentDraftFiles(key, [new File(['same'], 'same.mov', { type: 'video/quicktime' })])
    assert.ok(pending)
    resetRequestAttachmentDraftManager()

    const restored = readRequestAttachmentDraft(key)
    assert.equal(restored.text, 'full reload draft')
    assert.equal(restored.attachments[0]?.file, null)
    assert.equal(restored.attachments[0]?.operationId, pending.operationId)
    const [resumed] = addRequestAttachmentDraftFiles(key, [new File(['same'], 'same.mov', { type: 'video/quicktime' })])
    assert.equal(resumed?.localId, pending.localId)
    assert.equal(resumed?.operationId, pending.operationId)
  } finally {
    resetRequestAttachmentDraftManager()
    Object.defineProperty(globalThis, 'sessionStorage', { configurable: true, value: original })
  }
})

test('an access change clears only that repository draft and cancels its upload', () => {
  const oldScope = {
    accessScope: 'old-private', repoId: 'repo-one', requestId: 'request', target: 'discussion' as const, viewerId: 'viewer',
  }
  const otherScope = { ...oldScope, accessScope: 'other-private', repoId: 'repo-two' }
  const oldKey = requestAttachmentDraftKey(oldScope)
  const otherKey = requestAttachmentDraftKey(otherScope)
  setRequestAttachmentDraftText(oldKey, 'restricted draft')
  setRequestAttachmentDraftText(otherKey, 'other repository draft')
  const canceled: string[] = []
  registerRequestAttachmentDraftUploadCanceller((key) => canceled.push(key))

  try {
    activateRequestAttachmentDraftScope({
      accessScope: 'new-public',
      repoId: 'repo-one',
      viewerId: 'viewer',
    })

    assert.deepEqual(canceled, [oldKey])
    assert.equal(readRequestAttachmentDraft(oldKey).text, '')
    assert.equal(readRequestAttachmentDraft(otherKey).text, 'other repository draft')
  } finally {
    registerRequestAttachmentDraftUploadCanceller(() => {})
  }
})

test('draft retention evicts the oldest inactive editor', () => {
  for (let index = 0; index < 17; index += 1) {
    setRequestAttachmentDraftText(`draft-${index}`, `text-${index}`)
  }

  assert.equal(readRequestAttachmentDraft('draft-0').text, '')
  assert.equal(readRequestAttachmentDraft('draft-16').text, 'text-16')
})
