import { createApiClient } from '@/api/client'
import { parseRequestParams } from '@/api/request-inputs'
import type { RequestParams } from '@/api/types'
import {
  ApiRouteTemplates,
  buildApiPath,
  type RequestAttentionActionRequest,
} from '@/api/types.generated'
import { apiValidators } from '@/api/validators.generated'

export type RequestAttentionCommand =
  { action: 'claim' | 'release' | 'restore' | 'settle' } | { action: 'snooze'; until_unix: number }

export type RequestAttentionInput = RequestParams & RequestAttentionActionRequest

export function parseRequestAttentionInput(input: unknown): RequestAttentionInput {
  const params = parseRequestParams(input)
  const data = input as RequestAttentionInput
  const body =
    data.action === 'snooze'
      ? {
          action: data.action,
          expected_activity_version: data.expected_activity_version,
          until_unix: data.until_unix,
        }
      : { action: data.action, expected_activity_version: data.expected_activity_version }
  if (!apiValidators.RequestAttentionActionRequest(body))
    throw new Error('Request attention action is invalid.')
  return { ...params, ...body }
}

export function updateRequestAttentionForRequest(input: RequestAttentionInput) {
  const { owner, repo, request_id, ...body } = input
  return createApiClient().put(
    buildApiPath(ApiRouteTemplates.repoRequestAttention, { owner, repo, request_id }),
    apiValidators.RequestAttentionMutationResponse,
    { auth: 'required', body },
  )
}
