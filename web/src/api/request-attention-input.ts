import { parseRequestParams } from './request-inputs'
import type { RequestParams } from './types'
import type { RequestAttentionActionRequest } from './types.generated'
import { apiValidators } from './validators.generated'

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
