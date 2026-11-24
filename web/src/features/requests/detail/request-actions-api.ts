import { createApiClient } from '@/api/client'
import { requestRoute } from '@/api/paths'
import type { RequestParams } from '@/api/types'
import { ApiRouteTemplates, type RequestCloseReason } from '@/api/types.generated'
import {
  LeaveRequestResponseValidator,
  RequestCloseResponseValidator,
  RequestInviteeMutationResponseValidator,
  RequestMutationResponseValidator,
} from '@/api/validators.generated'

export type RequestActionCommand =
  | { action: 'add_invitee'; handle: string }
  | { action: 'close'; reason?: RequestCloseReason }
  | { action: 'leave' }
  | { action: 'release_work' }
  | { action: 'start_work' }
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
      const path = requestRoute(ApiRouteTemplates.repoRequest, input)
      const result = await api.delete(
        input.reason ? `${path}?${new URLSearchParams({ reason: input.reason })}` : path,
        RequestCloseResponseValidator,
        mutationOptions,
      )
      return { deleted: result.deleted }
    }
    case 'start_work':
      await api.post(
        requestRoute(ApiRouteTemplates.repoRequestWork, input),
        RequestMutationResponseValidator,
        mutationOptions,
      )
      return { deleted: false }
    case 'release_work':
      await api.delete(
        requestRoute(ApiRouteTemplates.repoRequestWork, input),
        RequestMutationResponseValidator,
        mutationOptions,
      )
      return { deleted: false }
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
