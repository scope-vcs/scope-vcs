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

export type AttachmentUploadTarget = {
  discussion_id: string | null
  kind: RequestAttachmentTargetKind
}

export type AttachmentPartReceipt = {
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
  if (target === 'discussion') {
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
  transfer: {
    acknowledged_parts: AttachmentPartReceipt[]
    grant: string
    media_base_url: string
    max_concurrent_parts: number
    preferred_part_bytes: number
    upload_id: string
    expires_at_unix: number
  }
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

  async function worker() {
    while (cursor < pending.length) {
      const partNumber = pending[cursor++]
      if (!partNumber) return
      assertActive(controller)
      if (Date.now() >= transfer.expires_at_unix * 1_000 - 30_000) {
        throw new TransferAuthorizationExpired()
      }
      const start = (partNumber - 1) * partBytes
      const part = file.slice(start, Math.min(file.size, start + partBytes))
      const receipt = await putPart({
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
        signal: controller.signal,
        url: `${safeMediaBaseUrl(transfer.media_base_url)}/v1/uploads/${encodeURIComponent(transfer.upload_id)}/parts/${partNumber}`,
      })
      inFlightBytes.delete(partNumber)
      receipts.set(partNumber, receipt)
      patchRequestAttachmentDraftFile(draftKey, localId, {
        progress: file.size === 0 ? 1 : Math.min(acknowledgedBytes() / file.size, 0.99),
      })
    }
  }

  const workerCount = Math.max(1, Math.min(transfer.max_concurrent_parts, pending.length || 1))
  const outcomes = await Promise.allSettled(
    Array.from({ length: workerCount }, () => worker()),
  )
  const failure = outcomes.find(
    (outcome): outcome is PromiseRejectedResult => outcome.status === 'rejected',
  )
  if (failure) throw failure.reason
  return [...receipts.values()].sort((left, right) => left.part_number - right.part_number)
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
        reject(request.status === 401
          ? new TransferAuthorizationExpired()
          : new Error(`Part ${partNumber} upload failed (${request.status}).`))
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
    request.addEventListener('error', () => reject(new Error(`Part ${partNumber} upload lost its connection.`)))
    request.addEventListener('abort', () => reject(new DOMException('Upload canceled.', 'AbortError')))
    request.send(part)
  })
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
