import {
  parseFinishAttachmentInput,
  parseGrantAttachmentInput,
  parsePrepareAttachmentInput,
  parseRequestParams,
  parseRetryAttachmentInput,
} from '@/api/request-inputs'
import {
  finishRequestAttachment,
  grantRequestAttachmentMedia,
  loadRequestAttachmentLimits,
  loadRequestAttachments,
  prepareRequestAttachment,
  retryRequestAttachment,
} from '@/features/requests/request-attachment-api'
import type { RequestAttachmentActions } from '@/features/requests/request-attachment-context'
import { createServerFn } from '@tanstack/react-start'

const listRequestAttachments = createServerFn({ method: 'GET' })
  .validator(parseRequestParams)
  .handler(({ data }) => loadRequestAttachments(data))

const loadAttachmentLimits = createServerFn({ method: 'GET' })
  .validator(parseRequestParams)
  .handler(({ data }) => loadRequestAttachmentLimits(data))

const prepareAttachment = createServerFn({ method: 'POST' })
  .validator(parsePrepareAttachmentInput)
  .handler(({ data }) => prepareRequestAttachment(data))

const finishAttachment = createServerFn({ method: 'POST' })
  .validator(parseFinishAttachmentInput)
  .handler(({ data }) => finishRequestAttachment(data))

const retryAttachment = createServerFn({ method: 'POST' })
  .validator(parseRetryAttachmentInput)
  .handler(({ data }) => retryRequestAttachment(data))

const grantAttachmentMedia = createServerFn({ method: 'POST' })
  .validator(parseGrantAttachmentInput)
  .handler(({ data }) => grantRequestAttachmentMedia(data))

export const requestAttachmentActions: RequestAttachmentActions = {
  finish: (data) => finishAttachment({ data }),
  grant: (data) => grantAttachmentMedia({ data }),
  limits: (data, signal) => loadAttachmentLimits({ data, signal }),
  list: (data, signal) => listRequestAttachments({ data, signal }),
  prepare: (data) => prepareAttachment({ data }),
  retry: (data) => retryAttachment({ data }),
}
