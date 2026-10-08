import { createApiClient } from '@/api/client'
import { requestRoute } from '@/api/paths'
import type { RequestParams } from '@/api/types'
import { ApiRouteTemplates } from '@/api/types.generated'
import {
  LeaveRequestResponseValidator,
  RequestCloseResponseValidator,
  RequestInviteeMutationResponseValidator,
  RequestMutationResponseValidator,
} from '@/api/validators.generated'

export type RequestActionCommand =
  | { action: 'add_invitee'; handle: string }
  | { action: 'close' }
  | { action: 'leave' }
  | { action: 'merge'; expected_head_oid: string }
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
        RequestMutationResponseValidator,
        { ...mutationOptions, body: {} },
      )
      return { deleted: false }
    case 'merge':
      await api.post(
        requestRoute(ApiRouteTemplates.repoRequestMerge, input),
        RequestMutationResponseValidator,
        { ...mutationOptions, body: { expected_head_oid: input.expected_head_oid } },
      )
      return { deleted: false }
    case 'close': {
      const result = await api.delete(
        requestRoute(ApiRouteTemplates.repoRequest, input),
        RequestCloseResponseValidator,
        mutationOptions,
      )
      return { deleted: result.deleted }
    }
    case 'add_invitee':
      await api.put(
        requestRoute(ApiRouteTemplates.repoRequestInvitees, input),
        RequestInviteeMutationResponseValidator,
        { ...mutationOptions, body: { handle: input.handle } },
      )
      return { deleted: false }
    case 'remove_invitee':
      await api.delete(
        requestRoute(ApiRouteTemplates.repoRequestInvitees, input),
        RequestInviteeMutationResponseValidator,
        { ...mutationOptions, body: { handle: input.handle } },
      )
      return { deleted: false }
    case 'leave':
      await api.delete(
        requestRoute(ApiRouteTemplates.repoRequestInviteesMe, input),
        LeaveRequestResponseValidator,
        mutationOptions,
      )
      return { deleted: false }
  }
}
