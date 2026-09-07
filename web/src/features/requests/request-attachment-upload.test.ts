import assert from 'node:assert/strict'
import test from 'node:test'
import {
  addRequestAttachmentDraftFiles,
  readRequestAttachmentDraft,
  requestAttachmentMarkdownReference,
  resetRequestAttachmentDraftManager,
  setRequestAttachmentDraftText,
} from './request-attachment-drafts'
import {
  type AttachmentUploadActions,
  uploadRequestAttachment,
} from './request-attachment-upload'

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
  authorization: string
  respond: (status: number, body: unknown) => void
  url: string
}

function installFakeXhr(
  send: (request: FakeRequest, part: Blob) => Promise<void>,
) {
  class FakeXMLHttpRequest {
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
      this.emit('abort')
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
