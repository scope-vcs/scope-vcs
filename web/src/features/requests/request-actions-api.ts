import { requestRoute } from '@/api/request-route'
import { createApiClient } from '@/api/client'
import type { RequestParams } from '@/api/types'
import { ApiRouteTemplates } from '@/api/types.generated'
import { apiValidators } from '@/api/validators.generated'

export type RequestActionCommand =
  | { action: 'add_invitee'; handle: string }
  | { action: 'close' }
  | { action: 'leave' }
  | { action: 'merge' }
  | { action: 'submit' }
  | { action: 'remove_invitee'; handle: string }

export type RequestActionInput = RequestParams & RequestActionCommand

export type RequestActionResult = {
  deleted: boolean
  synchronizationError?: string
}

export async function performRequestActionForRequest(
  input: RequestActionInput,
): Promise<RequestActionResult> {
  const api = createApiClient()
  const mutationOptions = { auth: 'required' as const }

  switch (input.action) {
    case 'submit':
      await api.post(
        requestRoute(ApiRouteTemplates.repoRequestSubmit, input),
        apiValidators.RequestMutationResponse,
        { ...mutationOptions, body: {} },
      )
      break
    case 'merge':
      await api.post(
        requestRoute(ApiRouteTemplates.repoRequestMerge, input),
        apiValidators.RequestMutationResponse,
        mutationOptions,
      )
      break
    case 'close': {
      const result = await api.delete(
        requestRoute(ApiRouteTemplates.repoRequest, input),
        apiValidators.RequestCloseResponse,
        mutationOptions,
      )
      return { deleted: result.deleted }
    }
    case 'add_invitee':
      await api.put(
        requestRoute(ApiRouteTemplates.repoRequestInvitees, input),
        apiValidators.RequestInviteeMutationResponse,
        { ...mutationOptions, body: { handle: input.handle } },
      )
      break
    case 'remove_invitee':
      await api.delete(
        requestRoute(ApiRouteTemplates.repoRequestInvitees, input),
        apiValidators.RequestInviteeMutationResponse,
        { ...mutationOptions, body: { handle: input.handle } },
      )
      break
    case 'leave':
      await api.delete(
        requestRoute(ApiRouteTemplates.repoRequestInviteesMe, input),
        apiValidators.LeaveRequestResponse,
        mutationOptions,
      )
      break
  }
  return { deleted: false }
}
