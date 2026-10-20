import type { RequestAttachmentTargetKind } from '@/api/types.generated'
import {
  patchRequestAttachmentDraftFile,
  readRequestAttachmentDraft,
  replaceRequestAttachmentDraftReference,
  removeRequestAttachmentDraftFile,
  registerRequestAttachmentDraftUploadCanceller,
  type RequestAttachmentDraftTarget,
} from './request-attachment-drafts'
import { hashAttachmentFile } from './request-attachment-hash-client'
import { HttpError } from '../../api/http'
import { resourceErrorMessage } from '../../lib/use-cached-resource'

export type AttachmentUploadParams = {
  owner: string
  repo: string
  request_id: string
}

type AttachmentUploadTarget = {
  discussion_id: string | null
  kind: RequestAttachmentTargetKind
}

type AttachmentPartReceipt = {
  part_number: number
  sha256: string
  size_bytes: number
}

export type AttachmentUploadActions = {
  finish: (input: AttachmentUploadParams & {
    attachment_id: string
    upload_id: string
    parts: AttachmentPartReceipt[]
  }) => Promise<{ id: string }>
  prepare: (input: AttachmentUploadParams & {
    declared_media_type: string
    filename: string
    operation_id: string
    sha256: string
    size_bytes: number
    target: AttachmentUploadTarget
  }) => Promise<{
    attachment: { id: string }
    transfer: {
      acknowledged_parts: AttachmentPartReceipt[]
      grant: string
      media_base_url: string
      max_concurrent_parts: number
      preferred_part_bytes: number
      upload_id: string
      expires_at_unix: number
    }
  }>
}

const activeUploads = new Map<string, AbortController>()
const MAX_TAB_PART_UPLOADS = 4
const PART_RETRY_BUDGET_MS = 90_000
const PART_RETRY_BASE_MS = 500
const PART_RETRY_CAP_MS = 10_000
const partUploadSlots = createSlots(MAX_TAB_PART_UPLOADS)

registerRequestAttachmentDraftUploadCanceller((draftKey) => {
  for (const [key, controller] of activeUploads) {
    if (!key.startsWith(`${draftKey}\0`)) continue
    controller.abort()
    activeUploads.delete(key)
  }
})

export async function uploadRequestAttachment({
  actions,
  draftKey,
  localId,
  params,
  target,
  onCompleted,
  hashFile = hashAttachmentFile,
}: {
  actions: AttachmentUploadActions
  draftKey: string
  localId: string
  params: AttachmentUploadParams
  target: AttachmentUploadTarget
  onCompleted?: () => void
  hashFile?: typeof hashAttachmentFile
}) {
  const attachment = readRequestAttachmentDraft(draftKey).attachments.find(
    (candidate) => candidate.localId === localId,
  )
  if (!attachment || !attachment.file || attachment.status === 'uploading') return
  const file = attachment.file
  const controller = new AbortController()
  activeUploads.set(uploadKey(draftKey, localId), controller)
  patchRequestAttachmentDraftFile(draftKey, localId, {
    error: null,
    progress: 0,
    status: 'uploading',
  })
  try {
    const sha256 = await hashFile(file, controller.signal)
    let operationId = attachment.sha256 && attachment.sha256 !== sha256
      ? crypto.randomUUID()
      : attachment.operationId
    patchRequestAttachmentDraftFile(draftKey, localId, {
      attachmentId: null,
      operationId,
      sha256,
    })
    assertActive(controller)
    let prepared: Awaited<ReturnType<AttachmentUploadActions['prepare']>> | null = null
    let receipts: AttachmentPartReceipt[] | null = null
    for (let authorizationAttempt = 0; authorizationAttempt < 3; authorizationAttempt += 1) {
      try {
        prepared = await actions.prepare({
          ...params,
          declared_media_type: attachment.contentType,
          filename: attachment.name,
          operation_id: operationId,
          sha256,
          size_bytes: file.size,
          target,
        })
      } catch (error) {
        if (!(error instanceof HttpError) || error.response.code !== 'attachment_upload_expired' || authorizationAttempt === 2) throw error
        assertActive(controller)
        operationId = crypto.randomUUID()
        patchRequestAttachmentDraftFile(draftKey, localId, { operationId })
        continue
      }
      assertActive(controller)
      try {
        receipts = await uploadParts({
          controller,
          draftKey,
          file,
          localId,
          transfer: prepared.transfer,
        })
        break
      } catch (error) {
        if (!(error instanceof TransferAuthorizationExpired) || authorizationAttempt === 2) throw error
      }
    }
    if (!prepared || !receipts) throw new Error('The upload authorization could not be renewed.')
    const completed = await actions.finish({
      ...params,
      attachment_id: prepared.attachment.id,
      parts: receipts,
      upload_id: prepared.transfer.upload_id,
    })
    assertActive(controller)
    patchRequestAttachmentDraftFile(draftKey, localId, {
      attachmentId: completed.id,
      error: null,
      progress: 1,
      status: 'uploaded',
    })
    replaceRequestAttachmentDraftReference(draftKey, localId, completed.id)
    onCompleted?.()
  } catch (error) {
    if (!controller.signal.aborted) {
      patchRequestAttachmentDraftFile(draftKey, localId, {
        error: resourceErrorMessage(error, 'The file could not be uploaded.'),
        status: 'failed',
      })
    }
  } finally {
    if (activeUploads.get(uploadKey(draftKey, localId)) === controller) {
      activeUploads.delete(uploadKey(draftKey, localId))
    }
  }
}

export function removeUploadingRequestAttachment(
  draftKey: string,
  localId: string,
) {
  activeUploads.get(uploadKey(draftKey, localId))?.abort()
  activeUploads.delete(uploadKey(draftKey, localId))
  removeRequestAttachmentDraftFile(draftKey, localId)
}

export function attachmentTargetForDraft(
  target: RequestAttachmentDraftTarget,
): AttachmentUploadTarget {
  if (target === 'description') {
    return { discussion_id: null, kind: 'Description' }
  }
  if (target === 'discussion' || target.startsWith('line-discussion:')) {
    return { discussion_id: null, kind: 'Discussion' }
  }
  return { discussion_id: target.slice('reply:'.length), kind: 'Reply' }
}

async function uploadParts({
  controller,
  draftKey,
  file,
  localId,
  transfer,
}: {
  controller: AbortController
  draftKey: string
  file: File
  localId: string
  transfer: Awaited<ReturnType<AttachmentUploadActions['prepare']>>['transfer']
}) {
  const partBytes = Math.max(1, transfer.preferred_part_bytes)
  const count = Math.ceil(file.size / partBytes)
  const receipts = new Map(
    transfer.acknowledged_parts.map((receipt) => [receipt.part_number, receipt]),
  )
  const acknowledgedBytes = () =>
    [...receipts.values()].reduce((total, receipt) => total + receipt.size_bytes, 0)
  const pending = Array.from({ length: count }, (_, index) => index + 1)
    .filter((partNumber) => !receipts.has(partNumber))
  let cursor = 0
  const inFlightBytes = new Map<number, number>()
  const partsController = new AbortController()
  const abortParts = () => partsController.abort()
  controller.signal.addEventListener('abort', abortParts, { once: true })
  if (controller.signal.aborted) partsController.abort()
  let failure: unknown = null

  async function worker() {
    try {
      while (cursor < pending.length) {
        const partNumber = pending[cursor++]
        if (!partNumber) return
        assertActive(partsController)
        if (Date.now() >= transfer.expires_at_unix * 1_000 - 30_000) {
          throw new TransferAuthorizationExpired()
        }
        const start = (partNumber - 1) * partBytes
        const part = file.slice(start, Math.min(file.size, start + partBytes))
        const receipt = await putPartWithRetry({
          grant: transfer.grant,
          onProgress: (loaded) => {
            inFlightBytes.set(partNumber, loaded)
            const completed = acknowledgedBytes()
            const uploading = [...inFlightBytes.values()].reduce((total, value) => total + value, 0)
            patchRequestAttachmentDraftFile(draftKey, localId, {
              progress: file.size === 0
                ? 1
                : Math.min((completed + uploading) / file.size, 0.99),
            })
          },
          part,
          partNumber,
          signal: partsController.signal,
          url: `${safeMediaBaseUrl(transfer.media_base_url)}/v1/uploads/${encodeURIComponent(transfer.upload_id)}/parts/${partNumber}`,
        })
        inFlightBytes.delete(partNumber)
        receipts.set(partNumber, receipt)
        patchRequestAttachmentDraftFile(draftKey, localId, {
          progress: file.size === 0 ? 1 : Math.min(acknowledgedBytes() / file.size, 0.99),
        })
      }
    } catch (error) {
      if (failure === null) {
        failure = error
        partsController.abort()
      }
    }
  }

  const workerCount = Math.max(1, Math.min(transfer.max_concurrent_parts, pending.length || 1, MAX_TAB_PART_UPLOADS))
  try {
    await Promise.all(Array.from({ length: workerCount }, () => worker()))
  } finally {
    controller.signal.removeEventListener('abort', abortParts)
  }
  if (failure !== null) throw failure
  return [...receipts.values()].sort((left, right) => left.part_number - right.part_number)
}

async function putPartWithRetry(input: Parameters<typeof putPart>[0]) {
  let firstFailureAt: number | null = null
  for (let retry = 0; ; retry += 1) {
    const release = await partUploadSlots.acquire(input.signal)
    try {
      input.signal.throwIfAborted()
      return await putPart(input)
    } catch (error) {
      if (!(error instanceof RetryablePartFailure)) throw error
      firstFailureAt ??= Date.now()
      if (Date.now() - firstFailureAt >= PART_RETRY_BUDGET_MS) throw error
    } finally {
      release()
    }
    input.onProgress(0)
    await wait(retryDelay(retry), input.signal)
  }
}

function retryDelay(retry: number) {
  const ceiling = Math.min(PART_RETRY_CAP_MS, PART_RETRY_BASE_MS * 2 ** retry)
  return ceiling / 2 + Math.random() * (ceiling / 2)
}

function wait(milliseconds: number, signal: AbortSignal) {
  return new Promise<void>((resolve, reject) => {
    signal.throwIfAborted()
    const abort = () => {
      clearTimeout(timeout)
      reject(new DOMException('Upload canceled.', 'AbortError'))
    }
    const timeout = setTimeout(() => {
      signal.removeEventListener('abort', abort)
      resolve()
    }, milliseconds)
    signal.addEventListener('abort', abort, { once: true })
  })
}

function createSlots(limit: number) {
  let active = 0
  const waiting: Array<() => void> = []
  function release() {
    const next = waiting.shift()
    if (next) next()
    else active -= 1
  }
  return {
    acquire(signal: AbortSignal) {
      signal.throwIfAborted()
      if (active < limit) {
        active += 1
        return Promise.resolve(release)
      }
      return new Promise<() => void>((resolve, reject) => {
        const grant = () => {
          signal.removeEventListener('abort', abort)
          resolve(release)
        }
        const abort = () => {
          waiting.splice(waiting.indexOf(grant), 1)
          reject(new DOMException('Upload canceled.', 'AbortError'))
        }
        waiting.push(grant)
        signal.addEventListener('abort', abort, { once: true })
      })
    },
  }
}

function putPart({
  grant,
  onProgress,
  part,
  partNumber,
  signal,
  url,
}: {
  grant: string
  onProgress: (loaded: number) => void
  part: Blob
  partNumber: number
  signal: AbortSignal
  url: string
}) {
  return new Promise<AttachmentPartReceipt>((resolve, reject) => {
    const request = new XMLHttpRequest()
    const abort = () => request.abort()
    signal.addEventListener('abort', abort, { once: true })
    request.open('PUT', url)
    request.setRequestHeader('authorization', `Bearer ${grant}`)
    request.setRequestHeader('content-type', 'application/octet-stream')
    request.upload.addEventListener('progress', (event) => onProgress(event.loaded))
    request.addEventListener('load', () => {
      signal.removeEventListener('abort', abort)
      if (request.status < 200 || request.status >= 300) {
        reject(partFailure(request.status, partNumber))
        return
      }
      try {
        const value: unknown = JSON.parse(request.responseText)
        if (!isPartReceipt(value)) throw new Error('The media service returned an invalid part receipt.')
        if (value.part_number !== partNumber || value.size_bytes !== part.size) {
          throw new Error('The media service acknowledged a different upload part.')
        }
        resolve(value)
      } catch (error) {
        reject(error)
      }
    })
    request.addEventListener('error', () => {
      signal.removeEventListener('abort', abort)
      reject(new RetryablePartFailure(`Part ${partNumber} upload lost its connection.`))
    })
    request.addEventListener('abort', () => reject(new DOMException('Upload canceled.', 'AbortError')))
    request.send(part)
  })
}

function partFailure(status: number, partNumber: number) {
  if (status === 401) return new TransferAuthorizationExpired()
  const message = `Part ${partNumber} upload failed (${status}).`
  return [408, 409, 429].includes(status) || status >= 500
    ? new RetryablePartFailure(message)
    : new Error(message)
}

class RetryablePartFailure extends Error {
  constructor(message: string) {
    super(message)
    this.name = 'RetryablePartFailure'
  }
}

class TransferAuthorizationExpired extends Error {
  constructor() {
    super('The upload authorization expired.')
    this.name = 'TransferAuthorizationExpired'
  }
}

function isPartReceipt(value: unknown): value is AttachmentPartReceipt {
  if (!value || typeof value !== 'object') return false
  const receipt = value as Record<string, unknown>
  return Number.isInteger(receipt.part_number) &&
    typeof receipt.sha256 === 'string' &&
    Number.isSafeInteger(receipt.size_bytes)
}

function assertActive(controller: AbortController) {
  if (controller.signal.aborted) throw new DOMException('Upload canceled.', 'AbortError')
}

function uploadKey(draftKey: string, localId: string) {
  return `${draftKey}\0${localId}`
}

function safeMediaBaseUrl(value: string) {
  const url = new URL(value)
  const local = url.protocol === 'http:' && ['127.0.0.1', 'localhost', '[::1]'].includes(url.hostname)
  if (url.protocol !== 'https:' && !local) throw new Error('The media service returned an unsafe upload URL.')
  if (url.username || url.password || url.pathname !== '/' || url.search || url.hash) {
    throw new Error('The media service returned an invalid upload URL.')
  }
  return url.origin
}
