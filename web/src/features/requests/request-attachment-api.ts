import { createApiClient } from '@/api/client'
import { requestRoute } from '@/api/paths'
import type { RequestParams } from '@/api/types'
import {
  ApiRouteTemplates,
  buildApiPath,
  type CreateRequestAttachmentMediaGrantRequest,
  type FinishRequestAttachmentRequest,
  type PrepareRequestAttachmentRequest,
  type RetryRequestAttachmentRequest,
} from '@/api/types.generated'
import { apiValidators } from '@/api/validators.generated'

export type RequestAttachmentActionInput = RequestParams & {
  attachment_id: string
}

export type PrepareAttachmentInput = RequestParams & PrepareRequestAttachmentRequest
export type FinishAttachmentInput = RequestAttachmentActionInput & FinishRequestAttachmentRequest
export type RetryAttachmentInput = RequestAttachmentActionInput & RetryRequestAttachmentRequest
export type GrantAttachmentInput = RequestAttachmentActionInput & CreateRequestAttachmentMediaGrantRequest

export function loadRequestAttachments(
  input: RequestParams,
  signal?: AbortSignal,
) {
  return createApiClient().get(
    requestRoute(ApiRouteTemplates.repoRequestAttachments, input),
    apiValidators.RequestAttachmentListResponse,
    { auth: 'optional', signal },
  )
}

export function loadRequestAttachmentLimits(
  input: RequestParams,
  signal?: AbortSignal,
) {
  return createApiClient().get(
    requestRoute(ApiRouteTemplates.repoRequestAttachmentLimits, input),
    apiValidators.RequestAttachmentLimitsResponse,
    { auth: 'optional', signal },
  )
}

export function prepareRequestAttachment(input: PrepareAttachmentInput) {
  return createApiClient().post(
    requestRoute(ApiRouteTemplates.repoRequestAttachmentPrepare, input),
    apiValidators.PrepareRequestAttachmentResponse,
    {
      auth: 'required',
      body: {
        declared_media_type: input.declared_media_type,
        filename: input.filename,
        operation_id: input.operation_id,
        sha256: input.sha256,
        size_bytes: input.size_bytes,
        target: input.target,
      },
    },
  )
}

export function finishRequestAttachment(input: FinishAttachmentInput) {
  return createApiClient().post(
    attachmentRoute(ApiRouteTemplates.repoRequestAttachmentFinish, input),
    apiValidators.RequestAttachmentResponse,
    {
      auth: 'required',
      body: { parts: input.parts, upload_id: input.upload_id },
    },
  )
}

export function retryRequestAttachment(input: RetryAttachmentInput) {
  return createApiClient().post(
    attachmentRoute(ApiRouteTemplates.repoRequestAttachmentRetry, input),
    apiValidators.RequestAttachmentResponse,
    { auth: 'required', body: { operation_id: input.operation_id } },
  )
}

export function grantRequestAttachmentMedia(input: GrantAttachmentInput) {
  return createApiClient().post(
    attachmentRoute(ApiRouteTemplates.repoRequestAttachmentMediaGrant, input),
    apiValidators.CreateRequestAttachmentMediaGrantResponse,
    { auth: 'optional', body: { target: input.target } },
  )
}

function attachmentRoute(template: string, input: RequestAttachmentActionInput) {
  return buildApiPath(template, {
    attachment_id: input.attachment_id,
    owner: input.owner,
    repo: input.repo,
    request_id: input.request_id,
  })
}
