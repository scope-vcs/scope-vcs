import { createApiClient } from '@/api/client'
import { requestRoute } from '@/api/paths'
import type { RequestAttentionInput } from '@/api/request-attention-input'
import { ApiRouteTemplates } from '@/api/types.generated'
import { apiValidators } from '@/api/validators.generated'

export type RequestAttentionCommand =
  { action: 'claim' | 'release' | 'restore' | 'settle' } | { action: 'snooze'; until_unix: number }

export function updateRequestAttentionForRequest(input: RequestAttentionInput) {
  const { owner, repo, request_id, ...body } = input
  return createApiClient().put(
    requestRoute(ApiRouteTemplates.repoRequestAttention, { owner, repo, request_id }),
    apiValidators.RequestAttentionMutationResponse,
    { auth: 'required', body },
  )
}
