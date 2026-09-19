import { createApiClient } from '@/api/client'
import { requestRoute } from '@/api/paths'
import type { RequestParams } from '@/api/types'
import type {
  AuthorizeRequestAutoMergeRequest,
  CancelRequestAutoMergeRequest,
  RequestAutoMergeResponse,
} from '@/api/types.generated'
import { ApiRouteTemplates } from '@/api/types.generated'
import { apiValidators } from '@/api/validators.generated'

export type AuthorizeRequestAutoMergeInput = RequestParams &
  AuthorizeRequestAutoMergeRequest

export type CancelRequestAutoMergeInput = RequestParams &
  CancelRequestAutoMergeRequest

export async function loadRequestAutoMergeForRequest(
  input: RequestParams,
  signal?: AbortSignal,
): Promise<RequestAutoMergeResponse> {
  return createApiClient().get(
    requestRoute(ApiRouteTemplates.repoRequestAutoMerge, input),
    apiValidators.RequestAutoMergeResponse,
    { auth: 'optional', signal },
  )
}

export async function authorizeRequestAutoMergeForRequest(
  input: AuthorizeRequestAutoMergeInput,
): Promise<RequestAutoMergeResponse> {
  return createApiClient().post(
    requestRoute(ApiRouteTemplates.repoRequestAutoMerge, input),
    apiValidators.RequestAutoMergeResponse,
    {
      auth: 'required',
      body: {
        expected_head_oid: input.expected_head_oid,
        expected_revision_id: input.expected_revision_id,
      } satisfies AuthorizeRequestAutoMergeRequest,
    },
  )
}

export async function cancelRequestAutoMergeForRequest(
  input: CancelRequestAutoMergeInput,
): Promise<RequestAutoMergeResponse> {
  return createApiClient().delete(
    requestRoute(ApiRouteTemplates.repoRequestAutoMerge, input),
    apiValidators.RequestAutoMergeResponse,
    {
      auth: 'required',
      body: {
        expected_intent_id: input.expected_intent_id,
      } satisfies CancelRequestAutoMergeRequest,
    },
  )
}
