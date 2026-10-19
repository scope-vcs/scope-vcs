import assert from 'node:assert/strict'
import test, { type TestContext } from 'node:test'
import { HttpError } from '../../api/http'
import {
  addRequestAttachmentDraftFiles,
  readRequestAttachmentDraft,
  requestAttachmentMarkdownReference,
  resetRequestAttachmentDraftManager,
  setRequestAttachmentDraftText,
} from './request-attachment-drafts'
import {
  type AttachmentUploadActions,
  removeUploadingRequestAttachment,
  uploadRequestAttachment,
} from './request-attachment-upload'

const hashFile = async () => 'a'.repeat(64)
const params = { owner: 'scope', repo: 'vcs', request_id: 'request-one' }
const target = { discussion_id: null, kind: 'Discussion' } as const

test.beforeEach(resetRequestAttachmentDraftManager)

test('uploads with the server concurrency bound and replaces the pending reference', async () => {
  const original = globalThis.XMLHttpRequest
  let inFlight = 0
  let peakInFlight = 0
  const uploadedParts: number[] = []
  installFakeXhr(async (request, part) => {
    inFlight += 1
    peakInFlight = Math.max(peakInFlight, inFlight)
    await new Promise((resolve) => setTimeout(resolve, 5))
    const partNumber = Number(request.url.split('/').at(-1))
    uploadedParts.push(partNumber)
    inFlight -= 1
    request.respond(200, {
      part_number: partNumber,
      sha256: `part-${partNumber}`,
      size_bytes: part.size,
    })
  })

  try {
    const key = 'multipart-draft'
    const [attachment] = addRequestAttachmentDraftFiles(
      key,
      [new File(['abcdefghijkl'], 'walkthrough.mp4', { type: 'video/mp4' })],
    )
    assert.ok(attachment)
    setRequestAttachmentDraftText(
      key,
      `[walkthrough](${requestAttachmentMarkdownReference(attachment.localId)})`,
    )
    let finishedParts: Array<{ part_number: number }> = []
    const actions: AttachmentUploadActions = {
      prepare: async () => ({
        attachment: { id: 'attachment-final' },
        transfer: {
          acknowledged_parts: [],
          expires_at_unix: Math.floor(Date.now() / 1_000) + 300,
          grant: 'upload-grant',
          max_concurrent_parts: 2,
          media_base_url: 'https://media.example.test',
          preferred_part_bytes: 4,
          upload_id: 'upload-one',
        },
      }),
      finish: async (input) => {
        finishedParts = input.parts
        return { id: 'attachment-final' }
      },
    }

    await uploadRequestAttachment({
      hashFile,
      actions,
      draftKey: key,
      localId: attachment.localId,
      params: { owner: 'scope', repo: 'vcs', request_id: 'request-one' },
      target: { discussion_id: null, kind: 'Discussion' },
    })

    assert.equal(peakInFlight, 2)
    assert.deepEqual(uploadedParts.sort(), [1, 2, 3])
    assert.deepEqual(finishedParts.map((part) => part.part_number), [1, 2, 3])
    const draft = readRequestAttachmentDraft(key)
    assert.equal(draft.attachments[0]?.status, 'uploaded')
    assert.equal(draft.attachments[0]?.attachmentId, 'attachment-final')
    assert.equal(
      draft.text,
      `[walkthrough](${requestAttachmentMarkdownReference('attachment-final')})`,
    )
  } finally {
    globalThis.XMLHttpRequest = original
  }
})

test('reauthorizes an expired transfer and reconciles acknowledged parts', async () => {
  const original = globalThis.XMLHttpRequest
  const grants: string[] = []
  installFakeXhr(async (request, part) => {
    const grant = request.authorization.replace('Bearer ', '')
    grants.push(grant)
    const partNumber = Number(request.url.split('/').at(-1))
    request.respond(grant === 'expired' ? 401 : 200, {
      part_number: partNumber,
      sha256: `part-${partNumber}`,
      size_bytes: part.size,
    })
  })

  try {
    const key = 'renewal-draft'
    const [attachment] = addRequestAttachmentDraftFiles(
      key,
      [new File(['abcdefgh'], 'screen.png', { type: 'image/png' })],
    )
    assert.ok(attachment)
    let prepareCalls = 0
    let finishCalls = 0
    const actions: AttachmentUploadActions = {
      prepare: async () => {
        prepareCalls += 1
        return {
          attachment: { id: 'attachment-renewed' },
          transfer: {
            acknowledged_parts: prepareCalls === 1
              ? []
              : [{ part_number: 1, sha256: 'part-1', size_bytes: 4 }],
            expires_at_unix: Math.floor(Date.now() / 1_000) + 300,
            grant: prepareCalls === 1 ? 'expired' : 'renewed',
            max_concurrent_parts: 1,
            media_base_url: 'https://media.example.test',
            preferred_part_bytes: 4,
            upload_id: 'upload-renewed',
          },
        }
      },
      finish: async (input) => {
        finishCalls += 1
        assert.deepEqual(input.parts.map((part) => part.part_number), [1, 2])
        return { id: 'attachment-renewed' }
      },
    }

    await uploadRequestAttachment({
      hashFile,
      actions,
      draftKey: key,
      localId: attachment.localId,
      params: { owner: 'scope', repo: 'vcs', request_id: 'request-one' },
      target: { discussion_id: null, kind: 'Discussion' },
    })

    assert.equal(prepareCalls, 2)
    assert.equal(finishCalls, 1)
    assert.deepEqual(grants, ['expired', 'renewed'])
    assert.equal(readRequestAttachmentDraft(key).attachments[0]?.status, 'uploaded')
  } finally {
    globalThis.XMLHttpRequest = original
  }
})

type FakeRequest = {
  aborted: boolean
  authorization: string
  fail: () => void
  respond: (status: number, body: unknown) => void
  url: string
}

test('rotates an expired operation and persists the replacement before retrying', async () => {
  const key = 'expired-operation-draft'
  const [attachment] = addRequestAttachmentDraftFiles(key, [new File(['data'], 'photo.png', { type: 'image/png' })])
  assert.ok(attachment)
  const operations: string[] = []
  const actions: AttachmentUploadActions = {
    prepare: async (input) => {
      operations.push(input.operation_id)
      if (operations.length === 1) throw new HttpError(409, {
        code: 'attachment_upload_expired', message: 'expired', retryable: false,
      })
      assert.equal(readRequestAttachmentDraft(key).attachments[0]?.operationId, input.operation_id)
      return {
        attachment: { id: 'fresh-attachment' },
        transfer: {
          acknowledged_parts: [{ part_number: 1, sha256: 'part-1', size_bytes: 4 }],
          expires_at_unix: Math.floor(Date.now() / 1000) + 300,
          grant: 'fresh', max_concurrent_parts: 1, preferred_part_bytes: 4,
          media_base_url: 'https://media.example.test', upload_id: 'fresh-upload',
        },
      }
    },
    finish: async () => ({ id: 'fresh-attachment' }),
  }
  await uploadRequestAttachment({
    actions, draftKey: key, localId: attachment.localId,
    params: { owner: 'scope', repo: 'vcs', request_id: 'request' },
    target: { discussion_id: null, kind: 'Description' },
    hashFile,
  })
  assert.equal(operations.length, 2)
  assert.notEqual(operations[0], operations[1])
  assert.equal(readRequestAttachmentDraft(key).attachments[0]?.status, 'uploaded')
})

function installFakeXhr(
  send: (request: FakeRequest, part: Blob) => Promise<void>,
) {
  class FakeXMLHttpRequest {
    aborted = false
    authorization = ''
    responseText = ''
    status = 0
    url = ''
    private listeners = new Map<string, Array<() => void>>()
    upload = {
      addEventListener: (_event: string, listener: (event: { loaded: number }) => void) => {
        this.progress = listener
      },
    }
    private progress: (event: { loaded: number }) => void = () => {}

    open(_method: string, url: string) {
      this.url = url
    }

    setRequestHeader(name: string, value: string) {
      if (name.toLowerCase() === 'authorization') this.authorization = value
    }

    addEventListener(event: string, listener: () => void) {
      const listeners = this.listeners.get(event) ?? []
      listeners.push(listener)
      this.listeners.set(event, listeners)
    }

    send(part: Blob) {
      this.progress({ loaded: part.size })
      void send(this, part)
    }

    abort() {
      this.aborted = true
      this.emit('abort')
    }

    fail() {
      this.emit('error')
    }

    respond(status: number, body: unknown) {
      this.status = status
      this.responseText = JSON.stringify(body)
      this.emit('load')
    }

    private emit(event: string) {
      for (const listener of this.listeners.get(event) ?? []) listener()
    }
  }

  globalThis.XMLHttpRequest = FakeXMLHttpRequest as unknown as typeof XMLHttpRequest
}

function partReceipt(request: FakeRequest, part: Blob) {
  const partNumber = Number(request.url.split('/').at(-1))
  return { part_number: partNumber, sha256: `part-${partNumber}`, size_bytes: part.size }
}

function singlePartActions(uploadId: string): AttachmentUploadActions {
  return {
    prepare: async () => ({
      attachment: { id: `${uploadId}-attachment` },
      transfer: {
        acknowledged_parts: [],
        expires_at_unix: Math.floor(Date.now() / 1_000) + 300,
        grant: 'grant',
        max_concurrent_parts: 2,
        media_base_url: 'https://media.example.test',
        preferred_part_bytes: 4,
        upload_id: uploadId,
      },
    }),
    finish: async () => ({ id: `${uploadId}-attachment` }),
  }
}

function addDraftFile(key: string, contents = 'abcd') {
  const [attachment] = addRequestAttachmentDraftFiles(key, [new File([contents], `${key}.png`, { type: 'image/png' })])
  assert.ok(attachment)
  return attachment.localId
}

async function settle() {
  for (let index = 0; index < 20; index += 1) await new Promise((resolve) => setImmediate(resolve))
}

function useFakeTimers(t: TestContext) {
  const original = globalThis.XMLHttpRequest
  t.after(() => { globalThis.XMLHttpRequest = original })
  t.mock.timers.enable({ apis: ['setTimeout', 'Date'] })
  t.mock.method(Math, 'random', () => 1)
}

const retryableFailures: Array<[string, (request: FakeRequest) => void]> = [
  ['busy gateway', (request) => request.respond(503, {})],
  ['stalled body', (request) => request.respond(408, {})],
  ['lost part lease', (request) => request.respond(409, {})],
  ['rate limit', (request) => request.respond(429, {})],
  ['server error', (request) => request.respond(500, {})],
  ['network error', (request) => request.fail()],
]

for (const [name, fail] of retryableFailures) {
  test(`retries a part after a ${name} with backoff`, async (t) => {
    useFakeTimers(t)
    let failed = false
    const send = t.mock.fn(async (request: FakeRequest, part: Blob) => {
      if (failed) return request.respond(200, partReceipt(request, part))
      failed = true
      fail(request)
    })
    installFakeXhr(send)
    const key = `retry-${name}`
    const localId = addDraftFile(key)

    const upload = uploadRequestAttachment({ actions: singlePartActions(key), draftKey: key, hashFile, localId, params, target })
    await settle()
    assert.equal(send.mock.callCount(), 1)
    t.mock.timers.tick(499)
    await settle()
    assert.equal(send.mock.callCount(), 1)
    t.mock.timers.tick(1)
    await upload

    assert.equal(send.mock.callCount(), 2)
    assert.equal(readRequestAttachmentDraft(key).attachments[0]?.status, 'uploaded')
  })
}

for (const status of [400, 403, 404, 413]) {
  test(`fails a part immediately on terminal status ${status}`, async (t) => {
    useFakeTimers(t)
    const send = t.mock.fn(async (request: FakeRequest) => request.respond(status, {}))
    installFakeXhr(send)
    const key = `terminal-${status}`
    const localId = addDraftFile(key)

    await uploadRequestAttachment({ actions: singlePartActions(key), draftKey: key, hashFile, localId, params, target })

    assert.equal(send.mock.callCount(), 1)
    const draft = readRequestAttachmentDraft(key).attachments[0]
    assert.equal(draft?.status, 'failed')
    assert.equal(draft?.error, `Part 1 upload failed (${status}).`)
  })
}

test('keeps retrying a part with capped backoff until its retry budget is spent', async (t) => {
  useFakeTimers(t)
  const attemptTimes: number[] = []
  const send = t.mock.fn(async (request: FakeRequest) => {
    attemptTimes.push(Date.now())
    request.respond(503, {})
  })
  installFakeXhr(send)
  const key = 'retry-exhausted'
  const localId = addDraftFile(key)

  const upload = uploadRequestAttachment({ actions: singlePartActions(key), draftKey: key, hashFile, localId, params, target })
  for (let elapsed = 0; elapsed < 120_000; elapsed += 500) {
    await settle()
    t.mock.timers.tick(500)
  }
  await upload

  const gaps = attemptTimes.slice(1).map((time, index) => time - attemptTimes[index]!)
  assert.deepEqual(gaps, [500, 1_000, 2_000, 4_000, 8_000, ...Array.from({ length: 8 }, () => 10_000)])
  assert.equal(send.mock.callCount(), 14)
  const draft = readRequestAttachmentDraft(key).attachments[0]
  assert.equal(draft?.status, 'failed')
  assert.equal(draft?.error, 'Part 1 upload failed (503).')
})

test('removing an upload during retry backoff stops further attempts', async (t) => {
  useFakeTimers(t)
  const send = t.mock.fn(async (request: FakeRequest) => request.respond(503, {}))
  installFakeXhr(send)
  const key = 'retry-cancelled'
  const localId = addDraftFile(key)

  const upload = uploadRequestAttachment({ actions: singlePartActions(key), draftKey: key, hashFile, localId, params, target })
  await settle()
  removeUploadingRequestAttachment(key, localId)
  await upload
  t.mock.timers.tick(60_000)
  await settle()

  assert.equal(send.mock.callCount(), 1)
  assert.deepEqual(readRequestAttachmentDraft(key).attachments, [])
})

test('caps part uploads across the tab and frees slots when uploads are cancelled', async (t) => {
  const original = globalThis.XMLHttpRequest
  t.after(() => { globalThis.XMLHttpRequest = original })
  const open: Array<{ part: Blob; request: FakeRequest }> = []
  let peakOpen = 0
  const send = t.mock.fn(async (request: FakeRequest, part: Blob) => {
    open.push({ part, request })
    peakOpen = Math.max(peakOpen, open.filter((entry) => !entry.request.aborted).length)
  })
  installFakeXhr(send)
  const respondToOpen = () => {
    for (const { part, request } of open.splice(0)) {
      if (!request.aborted) request.respond(200, partReceipt(request, part))
    }
  }
  const files = ['first', 'second', 'third', 'queued'].map((key) => ({ key, localId: addDraftFile(key, 'abcdefgh') }))
  const uploads = files.map(({ key, localId }) =>
    uploadRequestAttachment({ actions: singlePartActions(key), draftKey: key, hashFile, localId, params, target }))
  await settle()
  assert.equal(send.mock.callCount(), 4)

  removeUploadingRequestAttachment('first', files[0]!.localId)
  removeUploadingRequestAttachment('queued', files[3]!.localId)
  await settle()
  for (let round = 0; round < 4 && open.length > 0; round += 1) {
    respondToOpen()
    await settle()
  }
  await Promise.all(uploads)

  assert.equal(peakOpen, 4)
  assert.equal(send.mock.callCount(), 6)
  assert.equal(open.length, 0)
  assert.equal(readRequestAttachmentDraft('second').attachments[0]?.status, 'uploaded')
  assert.equal(readRequestAttachmentDraft('third').attachments[0]?.status, 'uploaded')
  assert.deepEqual(readRequestAttachmentDraft('first').attachments, [])
  assert.deepEqual(readRequestAttachmentDraft('queued').attachments, [])

  const later = ['later-one', 'later-two'].map((key) => ({ key, localId: addDraftFile(key, 'abcdefgh') }))
  const laterUploads = later.map(({ key, localId }) =>
    uploadRequestAttachment({ actions: singlePartActions(key), draftKey: key, hashFile, localId, params, target }))
  await settle()
  assert.equal(open.length, 4)
  respondToOpen()
  await Promise.all(laterUploads)
})
