import assert from 'node:assert/strict'
import test from 'node:test'
import {
  activateRequestAttachmentDraftScope,
  addRequestAttachmentDraftFiles,
  clearRequestAttachmentDraft,
  beginRequestAttachmentSubmission,
  finishRequestAttachmentSubmission,
  runRequestContentSubmission,
  seedRequestAttachmentDraft,
  setRequestAttachmentDraftReplyTarget,
  removeRequestAttachmentDraftFile,
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
  assert.deepEqual(readRequestAttachmentDraft(key), { attachments: [], baseText: null, initialized: false, pending: false, replyToReplyId: null, submission: null, text: '' })
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
    setRequestAttachmentDraftReplyTarget(key, 'quoted-reply')
    const submissionId = beginRequestAttachmentSubmission(key, 'payload')
    resetRequestAttachmentDraftManager()

    const restored = readRequestAttachmentDraft(key)
    assert.equal(restored.text, 'full reload draft')
    assert.equal(restored.pending, false)
    assert.equal(restored.replyToReplyId, 'quoted-reply')
    assert.equal(beginRequestAttachmentSubmission(key, 'payload'), submissionId)
    finishRequestAttachmentSubmission(submissionId!, false)
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


test('description drafts preserve their original base until explicitly discarded', () => {
  seedRequestAttachmentDraft('description', 'original server text')
  setRequestAttachmentDraftText('description', 'local edits')
  seedRequestAttachmentDraft('description', 'new server text')
  assert.equal(readRequestAttachmentDraft('description').baseText, 'original server text')
  assert.equal(readRequestAttachmentDraft('description').text, 'local edits')
  clearRequestAttachmentDraft('description')
  seedRequestAttachmentDraft('description', 'new server text')
  assert.equal(readRequestAttachmentDraft('description').baseText, 'new server text')
  assert.equal(readRequestAttachmentDraft('description').text, 'new server text')
})

test('pending submission locks survive reopening and reject text, file additions, and removal', () => {
  const key = 'locked'
  setRequestAttachmentDraftText(key, 'posted text')
  const [file] = addRequestAttachmentDraftFiles(key, [new File(['x'], 'photo.png')])
  assert.ok(file)
  const id = beginRequestAttachmentSubmission(key, 'payload')
  assert.ok(id)
  assert.equal(readRequestAttachmentDraft(key).pending, true)
  assert.equal(beginRequestAttachmentSubmission(key, 'payload'), null)
  setRequestAttachmentDraftText(key, 'unsent edit')
  assert.deepEqual(addRequestAttachmentDraftFiles(key, [new File(['y'], 'new.png')]), [])
  removeRequestAttachmentDraftFile(key, file.localId)
  assert.equal(readRequestAttachmentDraft(key).text, 'posted text')
  assert.equal(readRequestAttachmentDraft(key).attachments.length, 1)
  finishRequestAttachmentSubmission(id, false)
  assert.equal(readRequestAttachmentDraft(key).pending, false)
  assert.equal(beginRequestAttachmentSubmission(key, 'payload'), id)
  finishRequestAttachmentSubmission(id, true)
  assert.equal(readRequestAttachmentDraft(key).text, '')
})

test('composer retries and failed-row retries send one operation and clear the same draft', async () => {
  setRequestAttachmentDraftText('discussion', 'one discussion')
  const id = beginRequestAttachmentSubmission('discussion', 'same immutable payload')!
  let calls = 0
  assert.equal(await runRequestContentSubmission(id, async () => { calls += 1; return false }), false)
  assert.equal(beginRequestAttachmentSubmission('discussion', 'same immutable payload'), id)
  let complete!: (posted: boolean) => void
  const send = () => { calls += 1; return new Promise<boolean>((resolve) => { complete = resolve }) }
  const composerRetry = runRequestContentSubmission(id, send)
  const rowRetry = runRequestContentSubmission(id, send)
  assert.equal(composerRetry, rowRetry)
  await Promise.resolve()
  complete(true)
  assert.equal(await rowRetry, true)
  assert.equal(calls, 2)
  assert.equal(readRequestAttachmentDraft('discussion').text, '')
})

test('a failed older row cannot clear a newly edited draft and quote changes start a new attempt', async () => {
  setRequestAttachmentDraftText('reply', 'original')
  const oldId = beginRequestAttachmentSubmission('reply', 'original + quote-a')!
  finishRequestAttachmentSubmission(oldId, false)
  const changedQuoteId = beginRequestAttachmentSubmission('reply', 'original + quote-b')!
  assert.notEqual(changedQuoteId, oldId)
  finishRequestAttachmentSubmission(changedQuoteId, false)
  setRequestAttachmentDraftText('reply', 'new reply')
  await runRequestContentSubmission(oldId, async () => true)
  assert.equal(readRequestAttachmentDraft('reply').text, 'new reply')
  assert.notEqual(beginRequestAttachmentSubmission('reply', 'new reply + quote-b'), oldId)
})


test('quote targets survive closing and changing the quote resets the retry identity', () => {
  const key = 'reply-quote'
  setRequestAttachmentDraftText(key, 'reply text')
  setRequestAttachmentDraftReplyTarget(key, 'quote-a')
  const id = beginRequestAttachmentSubmission(key, 'same body')!
  setRequestAttachmentDraftReplyTarget(key, 'quote-b')
  assert.equal(readRequestAttachmentDraft(key).replyToReplyId, 'quote-a')
  finishRequestAttachmentSubmission(id, false)
  assert.equal(readRequestAttachmentDraft(key).replyToReplyId, 'quote-a')
  setRequestAttachmentDraftReplyTarget(key, 'quote-b')
  assert.notEqual(beginRequestAttachmentSubmission(key, 'same body'), id)
})
